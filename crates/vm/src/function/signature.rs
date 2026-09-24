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

#[derive(Clone, Copy, Debug)]
pub struct Param {
    pub name: &'static str,
    pub kind: ParamKind,
    pub default: Option<&'static str>,
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
    pub const fn keyword_only(name: &'static str, default: Option<&'static str>) -> Self {
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

const fn emit_named(buf: &mut [u8], st: St, prefix: &str, name: &str, default: Option<&str>) -> St {
    let st = emit_sep(buf, st);
    let n = put_str(buf, st.n, prefix);
    let n = put_str(buf, n, name);
    let n = if let Some(default) = default {
        let n = put_byte(buf, n, b'=');
        put_str(buf, n, default)
    } else {
        n
    };
    St {
        n,
        emitted: true,
        ..st
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
