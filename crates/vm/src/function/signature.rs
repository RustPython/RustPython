//! Compile-time text signatures for native functions.
//!
//! The internal doc is `name(params)\n--\n\ndoc`, built in a const context from
//! [`FromArgs`] metadata. Nothing here allocates.

use super::argument::FromArgs;

#[derive(Clone, Copy, Debug)]
pub enum ParamKind {
    PositionalOnly,
    PositionalOrKeyword,
    KeywordOnly,
    VarPositional,
    VarKeyword,
    /// Parameters contributed by a nested [`FromArgs`] type.
    /// `None` contributes nothing.
    Flatten(Option<&'static [Param]>),
}

/// Python source for a parameter default, chosen so it can be rendered in a const context.
#[derive(Clone, Copy, Debug)]
pub enum DefaultRepr {
    None,
    Bool(bool),
    Int(i128),
    Float(f64),
    Str(&'static str),
    Bytes(&'static [u8]),
    /// Verbatim Python source from `py_default`.
    Raw(&'static str),
    /// The argument may be omitted, and that state is not a Python value.
    Unrepresentable,
}

#[derive(Clone, Copy, Debug)]
pub struct Param {
    pub name: &'static str,
    pub kind: ParamKind,
    pub default: Option<DefaultRepr>,
}

impl Param {
    #[must_use]
    pub const fn positional_only(name: &'static str) -> Self {
        Self {
            name,
            kind: ParamKind::PositionalOnly,
            default: None,
        }
    }

    #[must_use]
    pub const fn positional_or_keyword(name: &'static str) -> Self {
        Self {
            name,
            kind: ParamKind::PositionalOrKeyword,
            default: None,
        }
    }

    #[must_use]
    pub const fn keyword_only(name: &'static str, default: Option<DefaultRepr>) -> Self {
        Self {
            name,
            kind: ParamKind::KeywordOnly,
            default,
        }
    }

    #[must_use]
    pub const fn var_positional(name: &'static str) -> Self {
        Self {
            name,
            kind: ParamKind::VarPositional,
            default: None,
        }
    }

    #[must_use]
    pub const fn var_keyword(name: &'static str) -> Self {
        Self {
            name,
            kind: ParamKind::VarKeyword,
            default: None,
        }
    }

    #[must_use]
    pub const fn flatten(params: Option<&'static [Self]>) -> Self {
        Self {
            name: "",
            kind: ParamKind::Flatten(params),
            default: None,
        }
    }
}

/// One Rust argument of a native function.
///
/// `params == None`: one positional-only parameter named [`name`](Self::name).
/// `Some(ps)`: the type supplies `ps` and `name` is ignored.
/// `Some(&[])`: the argument contributes nothing.
#[derive(Clone, Copy, Debug)]
pub struct SigArg {
    pub name: &'static str,
    pub params: Option<&'static [Param]>,
}

impl SigArg {
    #[must_use]
    pub const fn from_arg<T: FromArgs>(name: &'static str) -> Self {
        Self {
            name,
            params: T::PARAMS,
        }
    }

    /// `$self` / `$type` receiver marker. Renders as one positional-only parameter.
    #[must_use]
    pub const fn marker(name: &'static str) -> Self {
        Self { name, params: None }
    }

    /// Parameters supplied by the argument a method binds as its receiver.
    /// `None` metadata contributes nothing, so this never looks nameless.
    #[must_use]
    pub const fn implicit<T: FromArgs>() -> Self {
        Self {
            name: "",
            params: Some(match T::PARAMS {
                Some(ps) => ps,
                None => &[],
            }),
        }
    }
}

/// Calling-convention bits from each argument's binding metadata.
///
/// Inspect arguments separately: a tuple's `PARAMS` can erase leaf metadata.
/// With `receiver`, the first argument is the bound `$self` / `$type`; it
/// contributes only the parameters its type supplies.
/// A keyword-capable parameter uses `FASTCALL | KEYWORDS`.
#[must_use]
pub(crate) const fn native_call_flags(args: &[SigArg], receiver: bool) -> super::PyMethodFlags {
    let mut has_keywords = false;
    let mut variable = false;
    let mut fixed = 0usize;
    let mut i = 0;
    while i < args.len() {
        match args[i].params {
            Some(params) => count_params(params, &mut has_keywords, &mut variable, &mut fixed),
            None if receiver && i == 0 => {}
            None => fixed += 1,
        }
        i += 1;
    }
    if has_keywords {
        super::PyMethodFlags::FASTCALL.union(super::PyMethodFlags::KEYWORDS)
    } else if variable {
        super::PyMethodFlags::FASTCALL
    } else {
        match fixed {
            0 => super::PyMethodFlags::NOARGS,
            1 => super::PyMethodFlags::O,
            _ => super::PyMethodFlags::FASTCALL,
        }
    }
}

const fn count_params(
    params: &[Param],
    has_keywords: &mut bool,
    variable: &mut bool,
    fixed: &mut usize,
) {
    let mut i = 0;
    while i < params.len() {
        count_param(&params[i], has_keywords, variable, fixed);
        i += 1;
    }
}

const fn count_param(
    param: &Param,
    has_keywords: &mut bool,
    variable: &mut bool,
    fixed: &mut usize,
) {
    match param.kind {
        ParamKind::PositionalOrKeyword => {
            *has_keywords = true;
            if param.default.is_some() {
                *variable = true;
            } else {
                *fixed += 1;
            }
        }
        ParamKind::KeywordOnly | ParamKind::VarKeyword => *has_keywords = true,
        ParamKind::VarPositional => *variable = true,
        ParamKind::PositionalOnly => {
            if param.default.is_some() {
                *variable = true;
            } else {
                *fixed += 1;
            }
        }
        ParamKind::Flatten(Some(inner)) => count_params(inner, has_keywords, variable, fixed),
        ParamKind::Flatten(None) => {}
    }
}

const fn name_eq(name: &str, bytes: &[u8]) -> bool {
    let got = name.as_bytes();
    if got.len() != bytes.len() {
        return false;
    }
    let mut i = 0;
    while i < got.len() {
        if got[i] != bytes[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// `FuncArgs` contributes `*args, **kwargs`, which is not a callable's signature.
#[must_use]
const fn is_bare_funcargs(params: &[Param]) -> bool {
    params.len() == 2
        && matches!(params[0].kind, ParamKind::VarPositional)
        && name_eq(params[0].name, b"args")
        && matches!(params[1].kind, ParamKind::VarKeyword)
        && name_eq(params[1].name, b"kwargs")
}

const fn params_representable(params: &[Param]) -> bool {
    let mut i = 0;
    while i < params.len() {
        if let Some(DefaultRepr::Unrepresentable) = params[i].default {
            return false;
        }
        if let ParamKind::Flatten(Some(inner)) = params[i].kind
            && !params_representable(inner)
        {
            return false;
        }
        i += 1;
    }
    true
}

/// `None`, a bare `FuncArgs`, and a default of `<unrepresentable>` contribute
/// no signature. `Some(&[])` is `()`.
#[must_use]
pub const fn real_signature(params: Option<&[Param]>) -> Option<&[Param]> {
    match params {
        Some(ps) if !is_bare_funcargs(ps) && params_representable(ps) => Some(ps),
        _ => None,
    }
}

/// Prefer `preferred` when it is a real signature, otherwise `alternate`.
#[must_use]
pub const fn choose_class_params<'a>(
    preferred: Option<&'a [Param]>,
    alternate: Option<&'a [Param]>,
) -> Option<&'a [Param]> {
    match real_signature(preferred) {
        Some(ps) => Some(ps),
        None => real_signature(alternate),
    }
}

/// False when an argument is a destructured pattern whose type has no [`FromArgs::PARAMS`].
#[must_use]
pub const fn has_signature(args: &[SigArg]) -> bool {
    let mut i = 0;
    while i < args.len() {
        if args[i].params.is_none() && args[i].name.is_empty() {
            return false;
        }
        i += 1;
    }
    true
}

#[must_use]
pub const fn signature_prefix_len(name: &str, args: &[SigArg]) -> usize {
    write_signature_prefix(&mut [], name, args)
}

#[must_use]
pub const fn signature_prefix_bytes<const N: usize>(name: &str, args: &[SigArg]) -> [u8; N] {
    let mut buf = [0u8; N];
    let written = write_signature_prefix(&mut buf, name, args);
    assert!(written == N);
    buf
}

#[must_use]
pub const fn internal_doc_len(name: &str, args: &[SigArg], doc: &str) -> usize {
    write_internal_doc(&mut [], name, args, doc)
}

#[must_use]
pub const fn internal_doc_bytes<const N: usize>(name: &str, args: &[SigArg], doc: &str) -> [u8; N] {
    let mut buf = [0u8; N];
    let written = write_internal_doc(&mut buf, name, args, doc);
    assert!(written == N);
    buf
}

struct St {
    n: usize,
    emitted: bool,
    po_left: usize,
    var_pos_seen: bool,
    var_kw_seen: bool,
    star_emitted: bool,
}

const fn write_signature_prefix(buf: &mut [u8], name: &str, args: &[SigArg]) -> usize {
    let mut n = put_str(buf, 0, name);
    n = put_byte(buf, n, b'(');
    let st = write_args(
        buf,
        St {
            n,
            emitted: false,
            po_left: count_po_args(args),
            var_pos_seen: false,
            var_kw_seen: false,
            star_emitted: false,
        },
        args,
    );
    put_str(buf, st.n, ")\n--\n\n")
}

const fn write_internal_doc(buf: &mut [u8], name: &str, args: &[SigArg], doc: &str) -> usize {
    let n = write_signature_prefix(buf, name, args);
    put_str(buf, n, doc)
}

const fn put(buf: &mut [u8], i: usize, bytes: &[u8]) -> usize {
    let mut k = 0;
    while k < bytes.len() {
        let at = i + k;
        if at < buf.len() {
            buf[at] = bytes[k];
        }
        k += 1;
    }
    i + bytes.len()
}

const fn put_str(buf: &mut [u8], i: usize, s: &str) -> usize {
    put(buf, i, s.as_bytes())
}

const fn put_byte(buf: &mut [u8], i: usize, b: u8) -> usize {
    put(buf, i, &[b])
}

const fn count_po_params(params: &[Param]) -> usize {
    let mut n = 0;
    let mut i = 0;
    while i < params.len() {
        match params[i].kind {
            ParamKind::PositionalOnly => n += 1,
            ParamKind::Flatten(Some(inner)) => n += count_po_params(inner),
            _ => {}
        }
        i += 1;
    }
    n
}

const fn count_po_args(args: &[SigArg]) -> usize {
    let mut n = 0;
    let mut i = 0;
    while i < args.len() {
        match args[i].params {
            None => n += 1,
            Some(ps) => n += count_po_params(ps),
        }
        i += 1;
    }
    n
}

const fn emit_sep(buf: &mut [u8], st: St) -> St {
    if st.emitted {
        St {
            n: put_str(buf, st.n, ", "),
            ..st
        }
    } else {
        st
    }
}

const fn emit_text(buf: &mut [u8], st: St, text: &str) -> St {
    let st = emit_sep(buf, st);
    St {
        n: put_str(buf, st.n, text),
        emitted: true,
        ..st
    }
}

const fn emit_named(
    buf: &mut [u8],
    st: St,
    prefix: &str,
    name: &str,
    default: Option<DefaultRepr>,
) -> St {
    let st = emit_sep(buf, st);
    let n = put_str(buf, st.n, prefix);
    let n = put_str(buf, n, name);
    let n = if let Some(default) = default {
        let n = put_byte(buf, n, b'=');
        put_default(buf, n, default)
    } else {
        n
    };
    St {
        n,
        emitted: true,
        ..st
    }
}

const HEX: &[u8; 16] = b"0123456789abcdef";

const fn put_hex_byte(buf: &mut [u8], i: usize, b: u8) -> usize {
    let n = put_str(buf, i, "\\x");
    let n = put_byte(buf, n, HEX[(b >> 4) as usize]);
    put_byte(buf, n, HEX[(b & 0xf) as usize])
}

const fn put_quoted(buf: &mut [u8], mut i: usize, bytes: &[u8], utf8: bool) -> usize {
    i = put_byte(buf, i, b'\'');
    let mut k = 0;
    while k < bytes.len() {
        let b = bytes[k];
        if b == b'\\' {
            i = put_str(buf, i, "\\\\");
        } else if b == b'\'' {
            i = put_str(buf, i, "\\'");
        } else if b == b'\n' {
            i = put_str(buf, i, "\\n");
        } else if b == b'\r' {
            i = put_str(buf, i, "\\r");
        } else if b == b'\t' {
            i = put_str(buf, i, "\\t");
        } else if b < 0x20 || b == 0x7f || (!utf8 && b >= 0x80) {
            i = put_hex_byte(buf, i, b);
        } else {
            i = put_byte(buf, i, b);
        }
        k += 1;
    }
    put_byte(buf, i, b'\'')
}

const fn put_u128(buf: &mut [u8], i: usize, mut v: u128) -> usize {
    if v == 0 {
        return put_byte(buf, i, b'0');
    }
    let mut digits = [0u8; 40];
    let mut n = 0;
    while v > 0 {
        digits[n] = b'0' + (v % 10) as u8;
        v /= 10;
        n += 1;
    }
    let mut i = i;
    while n > 0 {
        n -= 1;
        i = put_byte(buf, i, digits[n]);
    }
    i
}

const fn put_i128(buf: &mut [u8], i: usize, v: i128) -> usize {
    if v < 0 {
        let i = put_byte(buf, i, b'-');
        put_u128(buf, i, (v as u128).wrapping_neg())
    } else {
        put_u128(buf, i, v as u128)
    }
}

const fn put_exp(buf: &mut [u8], i: usize, exp: i32) -> usize {
    let i = put_byte(buf, i, b'e');
    let (i, exp) = if exp < 0 {
        (put_byte(buf, i, b'-'), exp.wrapping_neg())
    } else {
        (put_byte(buf, i, b'+'), exp)
    };
    let exp = exp as u32;
    if exp >= 10 {
        let i = put_byte(buf, i, b'0' + (exp / 10) as u8);
        put_byte(buf, i, b'0' + (exp % 10) as u8)
    } else {
        let i = put_byte(buf, i, b'0');
        put_byte(buf, i, b'0' + exp as u8)
    }
}

/// Shortest round-trip decimal, matching `float.__repr__` for finite values.
const fn put_f64(buf: &mut [u8], i: usize, v: f64) -> usize {
    if v.is_nan() {
        return put_str(buf, i, "nan");
    }
    if v.is_infinite() {
        return put_str(buf, i, if v.is_sign_negative() { "-inf" } else { "inf" });
    }
    if v == 0.0 {
        return put_str(buf, i, if v.is_sign_negative() { "-0.0" } else { "0.0" });
    }
    let neg = v.is_sign_negative();
    let mut i = if neg { put_byte(buf, i, b'-') } else { i };
    let mut x = if neg { -v } else { v };
    let mut exp: i32 = 0;
    while x >= 10.0 && exp < 350 {
        x /= 10.0;
        exp += 1;
    }
    while x < 1.0 && exp > -350 {
        x *= 10.0;
        exp -= 1;
    }
    // 17 significant digits, then trim trailing zeros.
    let mut digits = [0u8; 17];
    let mut n = 0;
    while n < 17 {
        let d = x as u8;
        digits[n] = d;
        x = (x - d as f64) * 10.0;
        n += 1;
    }
    if x >= 5.0 {
        let mut k = 16;
        loop {
            if digits[k] < 9 {
                digits[k] += 1;
                break;
            }
            digits[k] = 0;
            if k == 0 {
                digits[0] = 1;
                exp += 1;
                break;
            }
            k -= 1;
        }
    }
    while n > 1 && digits[n - 1] == 0 {
        n -= 1;
    }
    let scientific = exp < -4 || exp >= 16;
    if scientific {
        i = put_byte(buf, i, b'0' + digits[0]);
        if n > 1 {
            i = put_byte(buf, i, b'.');
            let mut k = 1;
            while k < n {
                i = put_byte(buf, i, b'0' + digits[k]);
                k += 1;
            }
        }
        put_exp(buf, i, exp)
    } else if exp >= 0 {
        let exp_us = exp as usize;
        let mut k = 0;
        while k <= exp_us && k < n {
            i = put_byte(buf, i, b'0' + digits[k]);
            k += 1;
        }
        while k <= exp_us {
            i = put_byte(buf, i, b'0');
            k += 1;
        }
        i = put_byte(buf, i, b'.');
        if n as i32 > exp + 1 {
            let mut k = exp as usize + 1;
            while k < n {
                i = put_byte(buf, i, b'0' + digits[k]);
                k += 1;
            }
            i
        } else {
            put_byte(buf, i, b'0')
        }
    } else {
        i = put_str(buf, i, "0.");
        let mut z = 0;
        while z < -exp - 1 {
            i = put_byte(buf, i, b'0');
            z += 1;
        }
        let mut k = 0;
        while k < n {
            i = put_byte(buf, i, b'0' + digits[k]);
            k += 1;
        }
        i
    }
}

const fn put_default(buf: &mut [u8], i: usize, default: DefaultRepr) -> usize {
    match default {
        DefaultRepr::None => put_str(buf, i, "None"),
        DefaultRepr::Bool(true) => put_str(buf, i, "True"),
        DefaultRepr::Bool(false) => put_str(buf, i, "False"),
        DefaultRepr::Int(v) => put_i128(buf, i, v),
        DefaultRepr::Float(v) => put_f64(buf, i, v),
        DefaultRepr::Str(s) => put_quoted(buf, i, s.as_bytes(), true),
        DefaultRepr::Bytes(b) => {
            let i = put_byte(buf, i, b'b');
            put_quoted(buf, i, b, false)
        }
        DefaultRepr::Raw(s) => put_str(buf, i, s),
        DefaultRepr::Unrepresentable => put_str(buf, i, "<unrepresentable>"),
    }
}

const fn param_name<'a>(name: &'a str, fallback: &'a str) -> &'a str {
    if name.is_empty() { fallback } else { name }
}

const fn write_params(buf: &mut [u8], mut st: St, params: &[Param], fallback: &str) -> St {
    let mut i = 0;
    while i < params.len() {
        st = write_one(buf, st, params[i], fallback);
        i += 1;
    }
    st
}

const fn write_one(buf: &mut [u8], mut st: St, param: Param, fallback: &str) -> St {
    let name = param_name(param.name, fallback);
    match param.kind {
        ParamKind::Flatten(None) => st,
        ParamKind::Flatten(Some(inner)) => write_params(buf, st, inner, ""),
        ParamKind::PositionalOnly => {
            st = emit_named(buf, st, "", name, param.default);
            st.po_left -= 1;
            if st.po_left == 0 {
                // Python rejects `/` after `*` or `**`. The Rust arguments put a
                // keyword or variadic one before a positional one; reorder them.
                assert!(
                    !st.star_emitted && !st.var_pos_seen && !st.var_kw_seen,
                    "positional-only parameter after `*` or `**`: reorder the Rust arguments"
                );
                st = emit_text(buf, st, "/");
            }
            st
        }
        ParamKind::PositionalOrKeyword => emit_named(buf, st, "", name, param.default),
        ParamKind::KeywordOnly => {
            if !st.var_pos_seen && !st.star_emitted {
                st = emit_text(buf, st, "*");
                st.star_emitted = true;
            }
            emit_named(buf, st, "", name, param.default)
        }
        ParamKind::VarPositional => {
            st.var_pos_seen = true;
            emit_named(buf, st, "*", name, param.default)
        }
        ParamKind::VarKeyword => {
            st.var_kw_seen = true;
            emit_named(buf, st, "**", name, param.default)
        }
    }
}

const fn write_args(buf: &mut [u8], mut st: St, args: &[SigArg]) -> St {
    let mut i = 0;
    while i < args.len() {
        match args[i].params {
            None => {
                st = write_one(
                    buf,
                    st,
                    Param {
                        name: args[i].name,
                        kind: ParamKind::PositionalOnly,
                        default: None,
                    },
                    "",
                );
            }
            Some(ps) => st = write_params(buf, st, ps, args[i].name),
        }
        i += 1;
    }
    st
}

#[cfg(test)]
mod tests {
    use super::{DefaultRepr, Param, ParamKind, SigArg, St, native_call_flags, write_one};
    use crate::function::PyMethodFlags;

    fn rendered(default: DefaultRepr) -> String {
        let mut buf = [0u8; 64];
        let st = write_one(
            &mut buf,
            St {
                n: 0,
                emitted: false,
                po_left: 0,
                var_pos_seen: false,
                var_kw_seen: false,
                star_emitted: true,
            },
            Param {
                name: "x",
                kind: ParamKind::KeywordOnly,
                default: Some(default),
            },
            "",
        );
        let text = core::str::from_utf8(&buf[..st.n]).unwrap();
        text.split_once('=').unwrap().1.to_owned()
    }

    #[test]
    fn default_repr_text() {
        assert_eq!(rendered(DefaultRepr::None), "None");
        assert_eq!(rendered(DefaultRepr::Bool(true)), "True");
        assert_eq!(rendered(DefaultRepr::Bool(false)), "False");
        assert_eq!(rendered(DefaultRepr::Int(-15)), "-15");
        assert_eq!(rendered(DefaultRepr::Int(0)), "0");
        assert_eq!(rendered(DefaultRepr::Float(0.0)), "0.0");
        assert_eq!(rendered(DefaultRepr::Float(-1.0)), "-1.0");
        assert_eq!(rendered(DefaultRepr::Float(5.0)), "5.0");
        assert_eq!(rendered(DefaultRepr::Float(1e-9)), "1e-09");
        assert_eq!(rendered(DefaultRepr::Float(f64::INFINITY)), "inf");
        assert_eq!(rendered(DefaultRepr::Float(f64::NEG_INFINITY)), "-inf");
        assert_eq!(rendered(DefaultRepr::Str("a'b\n")), r"'a\'b\n'");
        assert_eq!(rendered(DefaultRepr::Bytes(b"a'b")), r"b'a\'b'");
        assert_eq!(rendered(DefaultRepr::Raw("sys.maxsize")), "sys.maxsize");
        assert_eq!(rendered(DefaultRepr::Unrepresentable), "<unrepresentable>");
    }

    #[test]
    fn native_call_flags_from_params() {
        const fn arg(params: Option<&'static [Param]>) -> SigArg {
            SigArg { name: "", params }
        }
        let keywords = PyMethodFlags::FASTCALL.union(PyMethodFlags::KEYWORDS);
        assert_eq!(native_call_flags(&[], false), PyMethodFlags::NOARGS);
        assert_eq!(native_call_flags(&[arg(None)], false), PyMethodFlags::O);
        assert_eq!(
            native_call_flags(&[arg(None), arg(None)], false),
            PyMethodFlags::FASTCALL
        );
        // The receiver is not a Python argument.
        assert_eq!(
            native_call_flags(&[SigArg::marker("$self"), arg(None)], true),
            PyMethodFlags::O
        );
        assert_eq!(native_call_flags(&[arg(None)], true), PyMethodFlags::NOARGS);

        const OPTIONAL: &[Param] = &[Param {
            name: "",
            kind: ParamKind::PositionalOnly,
            default: Some(DefaultRepr::Unrepresentable),
        }];
        assert_eq!(
            native_call_flags(&[arg(Some(OPTIONAL))], false),
            PyMethodFlags::FASTCALL
        );

        const KW: &[Param] = &[Param::positional_or_keyword("exp")];
        assert_eq!(native_call_flags(&[arg(Some(KW))], false), keywords);

        const FUNCARGS: &[Param] = &[Param::var_positional("args"), Param::var_keyword("kwargs")];
        assert_eq!(native_call_flags(&[arg(Some(FUNCARGS))], false), keywords);
        // A receiver that binds the whole call keeps its parameters.
        assert_eq!(native_call_flags(&[arg(Some(FUNCARGS))], true), keywords);

        const POSARGS: &[Param] = &[Param::var_positional("args")];
        assert_eq!(
            native_call_flags(&[arg(Some(POSARGS))], false),
            PyMethodFlags::FASTCALL
        );

        const INNER: &[Param] = &[Param::keyword_only(
            "reverse",
            Some(DefaultRepr::Bool(false)),
        )];
        const FLAT: &[Param] = &[Param::flatten(Some(INNER))];
        assert_eq!(native_call_flags(&[arg(Some(FLAT))], false), keywords);
    }
}
