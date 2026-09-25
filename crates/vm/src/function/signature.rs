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
    /// Verbatim Python source, including `py_default` text and `<unrepresentable>`.
    Raw(&'static str),
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
    star_emitted: bool,
}

const fn write_internal_doc(buf: &mut [u8], name: &str, args: &[SigArg], doc: &str) -> usize {
    let mut n = put_str(buf, 0, name);
    n = put_byte(buf, n, b'(');
    let st = write_args(
        buf,
        St {
            n,
            emitted: false,
            po_left: count_po_args(args),
            var_pos_seen: false,
            star_emitted: false,
        },
        args,
    );
    n = put_str(buf, st.n, ")\n--\n\n");
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
        ParamKind::VarKeyword => emit_named(buf, st, "**", name, param.default),
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
    use super::{DefaultRepr, Param, ParamKind, St, write_one};

    fn rendered(default: DefaultRepr) -> String {
        let mut buf = [0u8; 64];
        let st = write_one(
            &mut buf,
            St {
                n: 0,
                emitted: false,
                po_left: 0,
                var_pos_seen: false,
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
    }
}
