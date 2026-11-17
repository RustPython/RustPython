//! Compile-time text signatures for native functions.
//!
//! The internal doc is `name(params)\n--\n\ndoc`, built in a const context from
//! [`FromArgs`] metadata. Nothing here allocates.

use super::argument::{ArgsMetadata, FromArgs};

#[derive(Clone, Copy, Debug)]
pub enum ParamKind {
    PositionalOnly,
    PositionalOrKeyword,
    KeywordOnly,
    VarPositional,
    VarKeyword,
    /// Parameters contributed by a nested [`FromArgs`] type.
    /// `None` contributes nothing to display signatures. In binding metadata,
    /// it explicitly delegates validation to a custom parser.
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
    /// The Rust default has no declared Python representation.
    Unknown,
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
    /// Actual accepted arguments, independent of the displayed signature.
    /// `None` leaves dispatch permissive so a custom parser owns validation.
    pub binding_params: Option<&'static [Param]>,
    /// Force `_PyArg_UnpackKeywords` dispatch even for positional-only signatures.
    pub force_unpack_keywords: bool,
}

const SCALAR_PARAMS: &[Param] = &[Param::positional_only("")];

impl SigArg {
    #[must_use]
    pub const fn from_arg<T: ArgsMetadata>(name: &'static str) -> Self {
        Self {
            name,
            params: T::DISPLAY_PARAMS,
            binding_params: T::CLINIC_PARAMS,
            force_unpack_keywords: T::UNPACK_KEYWORDS,
        }
    }

    /// `$self` / `$type` receiver marker. Renders as one positional-only parameter.
    #[must_use]
    pub const fn marker(name: &'static str) -> Self {
        Self {
            name,
            params: None,
            binding_params: Some(SCALAR_PARAMS),
            force_unpack_keywords: false,
        }
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
            binding_params: T::BINDING_PARAMS,
            force_unpack_keywords: T::FORCE_UNPACK_KEYWORDS,
        }
    }
}

// CPython Objects/typeobject.c::signature_from_flags supplies these when a
// METH_NOARGS/METH_O function has no Clinic internal-doc signature. This is a
// display-only opt-in: module ownership and call dispatch remain unchanged.
#[must_use]
pub const fn signature_from_flags(args: &[SigArg]) -> &'static [SigArg] {
    const EMPTY: &[SigArg] = &[SigArg::marker("$self")];
    const ONE: &[SigArg] = &[SigArg::marker("$self"), SigArg::marker("object")];
    let mut display = 0;
    let mut binding = 0;
    let mut i = 0;
    while i < args.len() {
        display += match args[i].params {
            None => 1,
            Some(params) => flags_positional_count(params),
        };
        binding += match args[i].binding_params {
            Some(params) => flags_positional_count(params),
            None => panic!("signature_from_flags requires known argument binding"),
        };
        assert!(
            !args[i].force_unpack_keywords,
            "signature_from_flags cannot unpack keywords"
        );
        i += 1;
    }
    assert!(
        display == binding && binding <= 1,
        "signature_from_flags requires zero or one required positional-only argument"
    );
    if binding == 0 { EMPTY } else { ONE }
}

const fn flags_positional_count(params: &[Param]) -> usize {
    let mut count = 0;
    let mut i = 0;
    while i < params.len() {
        assert!(
            params[i].default.is_none(),
            "signature_from_flags cannot have optional arguments"
        );
        count += match params[i].kind {
            ParamKind::PositionalOnly => 1,
            ParamKind::Flatten(Some(inner)) => flags_positional_count(inner),
            _ => panic!("signature_from_flags requires known positional-only arguments"),
        };
        i += 1;
    }
    count
}

/// Check an explicit display signature against the actual argument binder.
///
/// Requiredness, arity and keyword names must agree. Receiver markers are
/// checked by the item macro, not by this user-argument validator. A
/// positional-only optional argument may be displayed as keyword-capable only
/// when the binder also accepts arbitrary keywords (legacy wrappers such as
/// `_csv` forward these to their subsequent parser). Defaults and names of
/// positional-only arguments are presentation, not conversion instructions.
/// Unknown/custom binders are deliberately rejected.
///
/// ```compile_fail
/// use rustpython_vm::function::{Param, SigArg, validate_text_signature};
/// const _: () = validate_text_signature(&[SigArg::marker("$self")], &[
///     Param::positional_only("$self"), Param::positional_only("extra"),
/// ]);
/// ```
///
/// An override cannot make a required parameter optional:
/// ```compile_fail
/// use rustpython_vm::function::{DefaultRepr, Param, ParamKind, SigArg, validate_text_signature};
/// const _: () = validate_text_signature(&[SigArg::marker("sub")], &[Param {
///     name: "sub", kind: ParamKind::PositionalOnly, default: Some(DefaultRepr::None),
/// }]);
/// ```
///
/// Documenting a forwarded keyword requires a keyword-accepting binder:
/// ```compile_fail
/// use rustpython_vm::function::{DefaultRepr, Param, ParamKind, SigArg, validate_text_signature};
/// const DIALECT: Param = Param {
///     name: "dialect", kind: ParamKind::PositionalOnly,
///     default: Some(DefaultRepr::Unrepresentable),
/// };
/// const _: () = validate_text_signature(&[SigArg {
///     name: "options", params: Some(&[DIALECT]), binding_params: Some(&[DIALECT]),
///     force_unpack_keywords: false,
/// }], &[Param { kind: ParamKind::PositionalOrKeyword, ..DIALECT }]);
/// ```
///
/// Unknown binders cannot establish that a display override is valid:
/// ```compile_fail
/// use rustpython_vm::function::{Param, SigArg, validate_text_signature};
/// const _: () = validate_text_signature(&[SigArg {
///     name: "custom", params: Some(&[Param::positional_only("value")]),
///     binding_params: None, force_unpack_keywords: false,
/// }], &[Param::positional_only("value")]);
/// ```
pub const fn validate_text_signature(args: &[SigArg], display: &[Param]) {
    validate_text_signature_impl(args, display, false);
}

/// Validate bracket notation that leaves parameter kinds to the binding.
pub const fn validate_text_signature_without_slash(args: &[SigArg], display: &[Param]) {
    validate_text_signature_impl(args, display, true);
}

// A known unrestricted parser can deliberately publish a narrower display
// contract, as CPython does for type.__prepare__. Unknown binders cannot.
const fn override_params(arg: &SigArg) -> &[Param] {
    let binding = match arg.binding_params {
        Some(params) => params,
        None => panic!("text_signature requires known binding metadata"),
    };
    if is_bare_funcargs(binding)
        && let Some(params) = arg.params
        && !is_bare_funcargs(params)
    {
        return params;
    }
    binding
}

const fn validate_text_signature_impl(args: &[SigArg], display: &[Param], omitted_slash: bool) {
    let mut count = 0;
    let mut var_keyword = false;
    let mut i = 0;
    while i < args.len() {
        let params = override_params(&args[i]);
        binding_override_count(params, &mut count, &mut var_keyword);
        i += 1;
    }
    assert!(
        count == display.len(),
        "text_signature parameter count differs from binding"
    );
    let mut index = 0;
    i = 0;
    while i < args.len() {
        validate_override_params(
            override_params(&args[i]),
            args[i].name,
            display,
            &mut index,
            var_keyword,
            omitted_slash,
        );
        i += 1;
    }
}

const fn binding_override_count(params: &[Param], count: &mut usize, keywords: &mut bool) {
    let mut i = 0;
    while i < params.len() {
        match params[i].kind {
            ParamKind::Flatten(Some(inner)) => binding_override_count(inner, count, keywords),
            ParamKind::Flatten(None) => panic!("text_signature requires known binding metadata"),
            ParamKind::VarKeyword => {
                *count += 1;
                *keywords = true;
            }
            _ => *count += 1,
        }
        i += 1;
    }
}

const fn validate_override_params(
    params: &[Param],
    fallback_name: &str,
    display: &[Param],
    index: &mut usize,
    var_keyword: bool,
    omitted_slash: bool,
) {
    let mut i = 0;
    while i < params.len() {
        let binding = &params[i];
        if let ParamKind::Flatten(Some(inner)) = binding.kind {
            validate_override_params(
                inner,
                fallback_name,
                display,
                index,
                var_keyword,
                omitted_slash,
            );
        } else {
            let shown = &display[*index];
            let name = if binding.name.is_empty() {
                fallback_name
            } else {
                binding.name
            };
            assert!(
                binding.default.is_some() == shown.default.is_some(),
                "text_signature changes requiredness"
            );
            let same_name = name_eq(name, shown.name.as_bytes());
            if omitted_slash {
                assert!(
                    matches!(
                        binding.kind,
                        ParamKind::PositionalOnly | ParamKind::PositionalOrKeyword
                    ),
                    "bracket signature requires positional parameters"
                );
                if matches!(binding.kind, ParamKind::PositionalOrKeyword) {
                    assert!(same_name, "text_signature changes keyword name");
                }
            } else {
                match (binding.kind, shown.kind) {
                    (ParamKind::PositionalOnly, ParamKind::PositionalOnly) => {}
                    (ParamKind::PositionalOnly, ParamKind::PositionalOrKeyword) => {
                        assert!(
                            var_keyword && binding.default.is_some() && same_name,
                            "text_signature keyword is not accepted by binding"
                        );
                    }
                    (ParamKind::PositionalOrKeyword, ParamKind::PositionalOrKeyword)
                    | (ParamKind::KeywordOnly, ParamKind::KeywordOnly) => {
                        assert!(same_name, "text_signature changes keyword name");
                    }
                    (ParamKind::VarKeyword, ParamKind::VarKeyword)
                    | (ParamKind::VarPositional, ParamKind::VarPositional) => {}
                    _ => panic!("text_signature parameter kind differs from binding"),
                }
            }
            *index += 1;
        }
        i += 1;
    }
}

/// Select the override of one complete argument spec. Composition cannot
/// silently discard an override or apply it to only part of a callable.
#[must_use]
pub const fn argument_text_signature(signatures: &[Option<&'static str>]) -> Option<&'static str> {
    let mut i = 0;
    while i < signatures.len() {
        if let Some(text) = signatures[i] {
            assert!(
                signatures.len() == 1,
                "text_signature requires a single complete argument spec"
            );
            return Some(text);
        }
        i += 1;
    }
    None
}

/// Length of an override, adding the receiver supplied by the item macro.
#[must_use]
pub const fn text_signature_len(signature: &str, receiver: Option<&str>) -> usize {
    signature.len()
        + match receiver {
            Some(receiver) => receiver.len() + if signature.len() == 2 { 3 } else { 2 },
            None => 0,
        }
}

/// Compose an already validated override with the selected documentation body.
#[must_use]
pub const fn text_signature_doc_bytes<const N: usize>(
    name: &str,
    signature: &str,
    receiver: Option<&str>,
    doc: &str,
) -> [u8; N] {
    let mut buf = [0; N];
    let n = put_str(&mut buf, 0, name);
    let n = if let Some(receiver) = receiver {
        let n = put_str(&mut buf, n, "(");
        let n = put_str(&mut buf, n, receiver);
        let n = put_str(&mut buf, n, if signature.len() == 2 { ", /" } else { ", " });
        let bytes = signature.as_bytes();
        let mut j = 1;
        let mut n = n;
        while j < bytes.len() {
            buf[n] = bytes[j];
            n += 1;
            j += 1;
        }
        n
    } else {
        put_str(&mut buf, n, signature)
    };
    let n = put_str(&mut buf, n, "\n--\n\n");
    let n = put_str(&mut buf, n, doc);
    assert!(n == N);
    buf
}

/// Calling-convention bits from each argument's binding metadata.
///
/// Display signatures must not prevent a custom parser from receiving a call.
/// Native receivers travel outside Python arguments and are excluded from this
/// metadata. A keyword-capable parameter uses `FASTCALL | KEYWORDS`.
#[must_use]
pub(crate) const fn native_call_flags(args: &[SigArg]) -> super::PyMethodFlags {
    let mut has_keywords = false;
    let mut variable = false;
    let mut fixed = 0usize;
    let mut i = 0;
    while i < args.len() {
        has_keywords |= args[i].force_unpack_keywords;
        match args[i].binding_params {
            Some(params) => count_params(params, &mut has_keywords, &mut variable, &mut fixed),
            None => {
                has_keywords = true;
                variable = true;
            }
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
        ParamKind::Flatten(None) => {
            *has_keywords = true;
            *variable = true;
        }
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
        if let Some(DefaultRepr::Unknown) = params[i].default {
            return false;
        }
        match params[i].kind {
            ParamKind::Flatten(Some(inner)) if !params_representable(inner) => return false,
            ParamKind::Flatten(_) => {}
            _ if params[i].name.is_empty() => return false,
            _ => {}
        }
        i += 1;
    }
    true
}

/// True when every parameter is positional-only or `*args`.
#[must_use]
pub(crate) const fn params_take_no_keywords(params: &[Param]) -> bool {
    let mut i = 0;
    while i < params.len() {
        match params[i].kind {
            ParamKind::PositionalOnly | ParamKind::VarPositional => {}
            ParamKind::Flatten(Some(inner)) => {
                if !params_take_no_keywords(inner) {
                    return false;
                }
            }
            ParamKind::Flatten(None)
            | ParamKind::PositionalOrKeyword
            | ParamKind::KeywordOnly
            | ParamKind::VarKeyword => return false,
        }
        i += 1;
    }
    true
}

/// `None`, a bare `FuncArgs`, and an unknown default contribute no signature.
/// A declared omitted value is rendered as `<unrepresentable>`, as in Clinic.
/// `Some(&[])` is `()`.
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

#[derive(Clone, Copy)]
struct St {
    n: usize,
    emitted: bool,
    po_left: usize,
    var_pos_seen: bool,
    var_kw_seen: bool,
    star_emitted: bool,
    // Layout is calculated only while constructing the static internal doc.
    indent: usize,
    column: usize,
    tokens: usize,
    total_tokens: usize,
}

const fn write_signature_prefix(buf: &mut [u8], name: &str, args: &[SigArg]) -> usize {
    let mut n = put_str(buf, 0, name);
    n = put_byte(buf, n, b'(');
    let indent = char_len(name) + 1;
    let initial = St {
        n,
        emitted: false,
        po_left: count_po_args(args),
        var_pos_seen: false,
        var_kw_seen: false,
        star_emitted: false,
        indent,
        column: indent,
        tokens: 0,
        total_tokens: usize::MAX,
    };
    // Clinic counts a token's trailing comma when deciding where to wrap.
    // The renderer runs twice at compile time. The counting pass includes synthetic
    // '/' and '*' tokens, without a separate traversal that could disagree with the
    // actual renderer.
    let total_tokens = write_args(&mut [], initial, args).tokens;
    let st = write_args(
        buf,
        St {
            total_tokens,
            ..initial
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

const fn char_len(text: &str) -> usize {
    let bytes = text.as_bytes();
    let mut count = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] & 0xc0 != 0x80 {
            count += 1;
        }
        i += 1;
    }
    count
}

const fn default_char_len(default: DefaultRepr) -> usize {
    let bytes = put_default(&mut [], 0, default);
    // Non-ASCII text is emitted verbatim for str/raw defaults; bytes defaults
    // instead escape every non-ASCII byte. Count characters as Clinic's len().
    match default {
        DefaultRepr::Str(text) | DefaultRepr::Raw(text) => bytes - text.len() + char_len(text),
        _ => bytes,
    }
}

const fn emit_sep(buf: &mut [u8], mut st: St, token_chars: usize) -> St {
    // Tools/clinic/libclinic/dsl_parser.py:add_parameter: wrap at >=72,
    // align with the first parameter, and never split a parameter/default.
    let trailing_comma = if st.tokens + 1 < st.total_tokens {
        1
    } else {
        0
    };
    if st.emitted {
        st.n = put_byte(buf, st.n, b',');
        st.column += 1;
        if st.column + 1 + token_chars + trailing_comma >= 72 {
            st.n = put_byte(buf, st.n, b'\n');
            let mut i = 0;
            while i < st.indent {
                st.n = put_byte(buf, st.n, b' ');
                i += 1;
            }
            st.column = st.indent;
        } else {
            st.n = put_byte(buf, st.n, b' ');
            st.column += 1;
        }
    }
    st.column += token_chars;
    st.tokens += 1;
    st
}

const fn emit_text(buf: &mut [u8], st: St, text: &str) -> St {
    let st = emit_sep(buf, st, char_len(text));
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
    let token_chars = char_len(prefix)
        + char_len(name)
        + match default {
            Some(value) => 1 + default_char_len(value),
            None => 0,
        };
    let st = emit_sep(buf, st, token_chars);
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
    let i = if exp < 10 { put_byte(buf, i, b'0') } else { i };
    put_u128(buf, i, exp as u128)
}

/// Decimal rendering using up to 17 significant digits.
// This normalization-based formatter is not a shortest-round-trip algorithm.
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
        DefaultRepr::Unrepresentable | DefaultRepr::Unknown => put_str(buf, i, "<unrepresentable>"),
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
    use super::{DefaultRepr, Param, ParamKind, SigArg, St, native_call_flags, put_exp, write_one};
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
                indent: 0,
                column: 0,
                tokens: 0,
                total_tokens: 1,
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

    fn layout(name: &str, params: &'static [Param]) -> String {
        let args = [SigArg {
            name: "options",
            params: Some(params),
            binding_params: Some(params),
            force_unpack_keywords: false,
        }];
        let mut buf = vec![0; super::signature_prefix_len(name, &args)];
        let written = super::write_signature_prefix(&mut buf, name, &args);
        assert_eq!(written, buf.len());
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn clinic_wraps_at_72_including_a_tokens_trailing_comma() {
        const A65: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        const A66: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        assert_eq!(
            layout(
                "f",
                const {
                    &[
                        Param::positional_or_keyword(A65),
                        Param::positional_or_keyword("bb"),
                    ]
                }
            ),
            format!("f({A65}, bb)\n--\n\n")
        );
        assert_eq!(
            layout(
                "f",
                const {
                    &[
                        Param::positional_or_keyword(A66),
                        Param::positional_or_keyword("bb"),
                    ]
                }
            ),
            format!("f({A66},\n  bb)\n--\n\n")
        );
        // The comma after bb moves the candidate from 71 to 72 columns.
        assert_eq!(
            layout(
                "f",
                const {
                    &[
                        Param::positional_or_keyword(A65),
                        Param::positional_or_keyword("bb"),
                        Param::positional_or_keyword("c"),
                    ]
                }
            ),
            format!("f({A65},\n  bb, c)\n--\n\n")
        );
        // The first parameter is never wrapped, even if already over width.
        assert_eq!(
            layout(
                "long_name",
                const {
                    &[
                        Param::positional_or_keyword(A66),
                        Param::positional_or_keyword("bb"),
                    ]
                }
            ),
            format!("long_name({A66},\n          bb)\n--\n\n")
        );
    }

    #[test]
    fn clinic_layout_preserves_default_contents_and_counts_unicode_characters() {
        // cspell:disable-next-line
        const UNICODE: &str = "ééééééééééééééééééééééééééééééééé";
        const ASCII: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        assert_eq!(
            layout(
                "f",
                const {
                    &[
                        Param::positional_or_keyword(UNICODE),
                        Param::positional_or_keyword(ASCII),
                    ]
                }
            ),
            format!("f({UNICODE}, {ASCII})\n--\n\n")
        );
        const TEXT: &str = "é,  space, \"quoted\", \\slash";
        const DEFAULT: Param = Param {
            name: "text",
            kind: ParamKind::PositionalOrKeyword,
            default: Some(DefaultRepr::Str(TEXT)),
        };
        assert_eq!(
            layout(
                "f",
                const {
                    &[
                        Param::positional_or_keyword(
                            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                        ),
                        DEFAULT,
                    ]
                }
            ),
            "f(aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa,\n  text='é,  space, \"quoted\", \\\\slash')\n--\n\n"
        );
        assert_eq!(super::default_char_len(DefaultRepr::Str("éé")), 4);
        assert_eq!(super::default_char_len(DefaultRepr::Raw("('é,  ', 1)")), 11);
    }

    #[test]
    fn clinic_layout_treats_slash_as_a_separate_token() {
        const ARGS: &[SigArg] = &[
            SigArg::marker("$module"),
            SigArg::marker("arraytype"),
            SigArg::marker("typecode"),
            SigArg::marker("mformat_code"),
            SigArg::marker("items"),
        ];
        const N: usize = super::signature_prefix_len("_array_reconstructor", ARGS);
        const TEXT: [u8; N] = super::signature_prefix_bytes::<N>("_array_reconstructor", ARGS);
        assert_eq!(
            core::str::from_utf8(&TEXT).unwrap(),
            "_array_reconstructor($module, arraytype, typecode, mformat_code, items,\n                     /)\n--\n\n"
        );
    }

    #[test]
    fn signatures_from_flags_use_noargs_and_o_fallbacks() {
        const NOARGS: &[SigArg] = super::signature_from_flags(&[]);
        const NOARGS_N: usize = super::signature_prefix_len("sample", NOARGS);
        const NOARGS_TEXT: [u8; NOARGS_N] =
            super::signature_prefix_bytes::<NOARGS_N>("sample", NOARGS);
        const ONE: &[SigArg] = super::signature_from_flags(&[SigArg::marker("custom_name")]);
        const ONE_N: usize = super::signature_prefix_len("sample", ONE);
        const ONE_TEXT: [u8; ONE_N] = super::signature_prefix_bytes::<ONE_N>("sample", ONE);
        assert_eq!(
            core::str::from_utf8(&NOARGS_TEXT).unwrap(),
            "sample($self, /)\n--\n\n"
        );
        assert_eq!(
            core::str::from_utf8(&ONE_TEXT).unwrap(),
            "sample($self, object, /)\n--\n\n"
        );
    }

    #[test]
    fn signature_from_flags_rejects_other_call_shapes() {
        const KEYWORD: &[Param] = &[Param::positional_or_keyword("value")];
        const OPTIONAL: &[Param] = &[Param {
            name: "value",
            kind: ParamKind::PositionalOnly,
            default: Some(DefaultRepr::None),
        }];
        const VARARGS: &[Param] = &[Param::var_positional("args")];
        const UNKNOWN: &[Param] = &[Param::flatten(None)];
        const POSITIONAL: &[Param] = &[Param::positional_only("value")];
        for params in [KEYWORD, OPTIONAL, VARARGS, UNKNOWN] {
            let args = [SigArg {
                name: "arg",
                params: Some(params),
                binding_params: Some(params),
                force_unpack_keywords: false,
            }];
            assert!(std::panic::catch_unwind(|| super::signature_from_flags(&args)).is_err());
        }
        assert!(
            std::panic::catch_unwind(|| super::signature_from_flags(&[
                SigArg::marker("one"),
                SigArg::marker("two")
            ]))
            .is_err()
        );
        let custom = SigArg {
            name: "arg",
            params: None,
            binding_params: None,
            force_unpack_keywords: false,
        };
        assert!(std::panic::catch_unwind(|| super::signature_from_flags(&[custom])).is_err());
        let mismatch = SigArg {
            name: "arg",
            params: Some(&[]),
            binding_params: Some(POSITIONAL),
            force_unpack_keywords: false,
        };
        assert!(std::panic::catch_unwind(|| super::signature_from_flags(&[mismatch])).is_err());
    }

    #[test]
    fn exponent_text_keeps_all_digits() {
        for (exp, expected) in [
            (0, "e+00"),
            (9, "e+09"),
            (-9, "e-09"),
            (10, "e+10"),
            (-99, "e-99"),
            (100, "e+100"),
            (308, "e+308"),
            (-324, "e-324"),
        ] {
            let mut buf = [0u8; 16];
            let len = put_exp(&mut buf, 0, exp);
            assert_eq!(core::str::from_utf8(&buf[..len]).unwrap(), expected);
            assert_eq!(put_exp(&mut [], 0, exp), len);
        }
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
        assert_eq!(rendered(DefaultRepr::Unknown), "<unrepresentable>");
    }

    #[test]
    fn explicit_signature_preserves_known_unrestricted_binding() {
        const DISPLAY: &[Param] = &[Param::positional_only("name"), Param::var_keyword("kwds")];
        const RAW: &[Param] = &[Param::var_positional("args"), Param::var_keyword("kwargs")];
        super::validate_text_signature(
            &[SigArg {
                name: "",
                params: Some(DISPLAY),
                binding_params: Some(RAW),
                force_unpack_keywords: false,
            }],
            DISPLAY,
        );
    }

    #[test]
    #[should_panic(expected = "text_signature changes keyword name")]
    fn bracket_override_cannot_rename_keyword() {
        const ACTUAL: &[Param] = &[Param::positional_or_keyword("actual")];
        super::validate_text_signature_without_slash(
            &[SigArg {
                name: "",
                params: Some(ACTUAL),
                binding_params: Some(ACTUAL),
                force_unpack_keywords: false,
            }],
            &[Param::positional_or_keyword("wrong")],
        );
    }

    #[test]
    fn class_signature_distinguishes_omission_from_unknown_default() {
        const OMITTED: &[Param] = &[Param::keyword_only(
            "name",
            Some(DefaultRepr::Unrepresentable),
        )];
        const UNKNOWN: &[Param] = &[Param::keyword_only("name", Some(DefaultRepr::Unknown))];
        assert!(super::real_signature(Some(OMITTED)).is_some());
        assert!(super::real_signature(Some(UNKNOWN)).is_none());
        assert!(super::real_signature(Some(&[Param::flatten(Some(OMITTED))])).is_some());
        assert!(super::real_signature(Some(&[Param::flatten(Some(UNKNOWN))])).is_none());
        const UNNAMED: &[Param] = &[Param::keyword_only("", Some(DefaultRepr::Unrepresentable))];
        assert!(super::real_signature(Some(UNNAMED)).is_none());
        assert!(super::real_signature(Some(&[Param::flatten(Some(UNNAMED))])).is_none());
    }

    #[test]
    fn native_call_flags_from_params() {
        const fn arg(params: Option<&'static [Param]>) -> SigArg {
            SigArg {
                name: "",
                params,
                binding_params: match params {
                    Some(params) => Some(params),
                    None => Some(super::SCALAR_PARAMS),
                },
                force_unpack_keywords: false,
            }
        }
        let keywords = PyMethodFlags::FASTCALL.union(PyMethodFlags::KEYWORDS);
        assert_eq!(native_call_flags(&[]), PyMethodFlags::NOARGS);
        assert_eq!(native_call_flags(&[arg(None)]), PyMethodFlags::O);
        assert_eq!(
            native_call_flags(&[arg(None), arg(None)]),
            PyMethodFlags::FASTCALL
        );

        const OPTIONAL: &[Param] = &[Param {
            name: "",
            kind: ParamKind::PositionalOnly,
            default: Some(DefaultRepr::Unrepresentable),
        }];
        assert_eq!(
            native_call_flags(&[arg(Some(OPTIONAL))]),
            PyMethodFlags::FASTCALL
        );

        const KW: &[Param] = &[Param::positional_or_keyword("exp")];
        assert_eq!(native_call_flags(&[arg(Some(KW))]), keywords);

        const FUNCARGS: &[Param] = &[Param::var_positional("args"), Param::var_keyword("kwargs")];
        assert_eq!(native_call_flags(&[arg(Some(FUNCARGS))]), keywords);

        const POSARGS: &[Param] = &[Param::var_positional("args")];
        assert_eq!(
            native_call_flags(&[arg(Some(POSARGS))]),
            PyMethodFlags::FASTCALL
        );

        const INNER: &[Param] = &[Param::keyword_only(
            "reverse",
            Some(DefaultRepr::Bool(false)),
        )];
        const FLAT: &[Param] = &[Param::flatten(Some(INNER))];
        assert_eq!(native_call_flags(&[arg(Some(FLAT))]), keywords);
    }

    #[test]
    fn unpack_keywords_keeps_the_positional_signature_and_keyword_dispatch() {
        type Wrapped = crate::function::UnpackKeywords<crate::PyObjectRef>;
        const ARGS: &[SigArg] = &[
            SigArg::marker("$self"),
            SigArg::from_arg::<Wrapped>("callback"),
        ];
        const FLAGS: PyMethodFlags = native_call_flags(&[SigArg::from_arg::<Wrapped>("callback")]);
        assert_eq!(
            FLAGS,
            PyMethodFlags::FASTCALL.union(PyMethodFlags::KEYWORDS)
        );
        const N: usize = super::signature_prefix_len("example", ARGS);
        const TEXT: [u8; N] = super::signature_prefix_bytes::<N>("example", ARGS);
        assert_eq!(&TEXT, b"example($self, callback, /)\n--\n\n");
    }

    #[test]
    fn dispatch_uses_binding_overrides_without_changing_display_signatures() {
        const ARGS: &[SigArg] = &[SigArg {
            name: "ignored",
            params: Some(&[]),
            binding_params: Some(&[Param::var_positional("args"), Param::var_keyword("kwargs")]),
            force_unpack_keywords: false,
        }];
        let keywords = PyMethodFlags::FASTCALL.union(PyMethodFlags::KEYWORDS);
        assert_eq!(native_call_flags(ARGS), keywords);
        const N: usize = super::signature_prefix_len("example", ARGS);
        const TEXT: [u8; N] = super::signature_prefix_bytes::<N>("example", ARGS);
        assert_eq!(&TEXT, b"example()\n--\n\n");
    }

    #[test]
    fn explicit_custom_parsers_receive_keywords_even_when_nested() {
        const CUSTOM: SigArg = SigArg {
            name: "custom",
            params: Some(&[Param::positional_only("display")]),
            binding_params: None,
            force_unpack_keywords: false,
        };
        const NESTED: SigArg = SigArg {
            name: "nested",
            params: Some(&[]),
            binding_params: Some(&[Param::flatten(None)]),
            force_unpack_keywords: false,
        };
        let keywords = PyMethodFlags::FASTCALL.union(PyMethodFlags::KEYWORDS);
        assert_eq!(native_call_flags(&[CUSTOM]), keywords);
        assert_eq!(native_call_flags(&[NESTED]), keywords);
    }
    const OPTIONAL_START: Param = Param {
        name: "start",
        kind: ParamKind::PositionalOnly,
        default: Some(DefaultRepr::None),
    };
    const OPTIONAL_END: Param = Param {
        name: "end",
        ..OPTIONAL_START
    };
    const SEARCH_BINDING: &[SigArg] = &[SigArg {
        name: "options",
        params: Some(&[]),
        binding_params: Some(&[
            Param::positional_only("affix"),
            Param::flatten(Some(&[OPTIONAL_START, OPTIONAL_END])),
        ]),
        force_unpack_keywords: false,
    }];
    const SEARCH_DISPLAY: &[Param] = &[
        Param::positional_only("prefix"),
        OPTIONAL_START,
        OPTIONAL_END,
    ];
    const DIALECT: Param = Param {
        name: "dialect",
        default: Some(DefaultRepr::Unrepresentable),
        ..OPTIONAL_START
    };
    const CSV_BINDING: &[SigArg] = &[
        SigArg::marker("iterable"),
        SigArg {
            name: "options",
            params: Some(&[]),
            binding_params: Some(&[
                DIALECT,
                Param::flatten(Some(&[Param::var_keyword("kwargs")])),
            ]),
            force_unpack_keywords: false,
        },
    ];
    const CSV_DISPLAY: &[Param] = &[
        Param::positional_only("iterable"),
        Param {
            kind: ParamKind::PositionalOrKeyword,
            default: Some(DefaultRepr::Str("excel")),
            ..DIALECT
        },
        Param::var_keyword("fmtparams"),
    ];
    const _: () = super::validate_text_signature(SEARCH_BINDING, SEARCH_DISPLAY);
    const _: () = super::validate_text_signature(CSV_BINDING, CSV_DISPLAY);

    #[test]
    fn argument_spec_overrides_are_shared_without_changing_binding() {
        use crate::function::{ArgsMetadata, FromArgs};
        type Prefix = crate::anystr::StartsEndsWithArgs;
        type Suffix = crate::anystr::EndsWithArgs;
        assert_eq!(
            <Prefix as FromArgs>::TEXT_SIGNATURE,
            Some("(prefix[, start[, end]], /)")
        );
        assert_eq!(
            <Suffix as FromArgs>::TEXT_SIGNATURE,
            Some("(suffix[, start[, end]], /)")
        );
        assert_eq!(Prefix::arity(), Suffix::arity());
        assert_eq!(
            native_call_flags(&[SigArg::from_arg::<Prefix>("")]),
            native_call_flags(&[SigArg::from_arg::<Suffix>("")])
        );
        #[derive(crate::FromArgs)]
        #[from_args(borrowed, text_signature = "(value, /)")]
        struct Borrowed<'a> {
            #[pyarg(positional)]
            value: &'a crate::PyObject,
        }
        assert_eq!(
            <Borrowed<'static> as ArgsMetadata>::TEXT_SIGNATURE,
            Some("(value, /)")
        );
        // Keep the fixture's payload observable without constructing an interpreter.
        fn value(args: Borrowed<'_>) -> &crate::PyObject {
            args.value
        }
        let _ = value;
    }

    #[test]
    fn shared_override_composition_adds_only_the_item_receiver() {
        const TEXT: &str = "(prefix[, start[, end]], /)";
        const N: usize = "startswith".len() + super::text_signature_len(TEXT, Some("$self")) + 5;
        const DOC: [u8; N] =
            super::text_signature_doc_bytes::<N>("startswith", TEXT, Some("$self"), "");
        assert_eq!(
            &DOC,
            b"startswith($self, prefix[, start[, end]], /)\n--\n\n"
        );
        const EMPTY_LEN: usize = 1 + super::text_signature_len("()", Some("$module")) + 5;
        const EMPTY: [u8; EMPTY_LEN] =
            super::text_signature_doc_bytes::<EMPTY_LEN>("f", "()", Some("$module"), "");
        assert_eq!(&EMPTY, b"f($module, /)\n--\n\n");
        assert_eq!(super::argument_text_signature(&[Some(TEXT)]), Some(TEXT));
        assert_eq!(super::argument_text_signature(&[None, None]), None);
        assert!(
            std::panic::catch_unwind(|| super::argument_text_signature(&[None, Some(TEXT)]))
                .is_err()
        );
        assert!(
            std::panic::catch_unwind(|| super::argument_text_signature(&[Some(TEXT), None]))
                .is_err()
        );
    }

    #[test]
    fn text_signature_overrides_preserve_binding_and_doc_body() {
        assert_eq!(native_call_flags(SEARCH_BINDING), PyMethodFlags::FASTCALL);
        assert_eq!(
            native_call_flags(CSV_BINDING),
            PyMethodFlags::FASTCALL.union(PyMethodFlags::KEYWORDS)
        );
        const TEXT: &str = "($self, prefix[, start[, end]], /)";
        const BODY: &str = "Return whether the string starts with prefix.\n";
        const N: usize = "startswith".len() + TEXT.len() + 5 + BODY.len();
        const DOC: [u8; N] = super::text_signature_doc_bytes::<N>("startswith", TEXT, None, BODY);
        assert_eq!(
            core::str::from_utf8(&DOC).unwrap(),
            format!("startswith{TEXT}\n--\n\n{BODY}")
        );
        const PREFIX: [u8; N - BODY.len()] =
            super::text_signature_doc_bytes::<{ N - BODY.len() }>("startswith", TEXT, None, "");
        assert_eq!(
            core::str::from_utf8(&PREFIX).unwrap(),
            format!("startswith{TEXT}\n--\n\n")
        );
    }

    #[test]
    fn text_signature_overrides_reject_incompatible_binding() {
        let rejected = |args: &[SigArg], display: &[Param]| {
            assert!(
                std::panic::catch_unwind(|| super::validate_text_signature(args, display)).is_err()
            );
        };
        rejected(SEARCH_BINDING, &SEARCH_DISPLAY[..2]);
        let mut display = SEARCH_DISPLAY.to_vec();
        display.push(Param::positional_only("extra"));
        rejected(SEARCH_BINDING, &display);
        display.pop();
        display[0].default = Some(DefaultRepr::None);
        rejected(SEARCH_BINDING, &display);
        display[0] = SEARCH_DISPLAY[0];
        display[1].default = None;
        rejected(SEARCH_BINDING, &display);
        display[1] = Param {
            kind: ParamKind::PositionalOrKeyword,
            ..OPTIONAL_START
        };
        rejected(SEARCH_BINDING, &display);
        let mut display = CSV_DISPLAY.to_vec();
        display[1].name = "wrong_keyword";
        rejected(CSV_BINDING, &display);
        display[1] = CSV_DISPLAY[1];
        display[2] = Param::var_positional("args");
        rejected(CSV_BINDING, &display);
        const NAMED: SigArg = SigArg {
            name: "options",
            params: Some(&[]),
            binding_params: Some(&[Param::positional_or_keyword("named")]),
            force_unpack_keywords: false,
        };
        rejected(&[NAMED], &[Param::positional_or_keyword("renamed")]);
        rejected(&[NAMED], &[Param::positional_only("named")]);
        rejected(
            &[SigArg {
                binding_params: None,
                ..NAMED
            }],
            &[Param::positional_or_keyword("named")],
        );
        const UNKNOWN: &[Param] = &[Param::flatten(None)];
        rejected(
            &[SigArg {
                binding_params: Some(UNKNOWN),
                ..NAMED
            }],
            &[Param::positional_or_keyword("named")],
        );
    }
}
