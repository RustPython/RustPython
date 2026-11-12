//! Explicit legacy text signatures. This deliberately accepts a small grammar:
//! positional parameters, a `/`, literal defaults, `**kwargs`, and trailing
//! nested optional positional groups. It is not a Python expression parser.
use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::{Error, Result};

pub(crate) struct TextSignature {
    pub(crate) text: String,
    params: Vec<Parameter>,
    pub(crate) omitted_slash: bool,
}

struct Parameter {
    name: String,
    positional: bool,
    optional: bool,
    keywords: bool,
}

fn identifier(name: &str) -> bool {
    let name = name.strip_prefix('$').unwrap_or(name);
    if matches!(
        name,
        "False"
            | "None"
            | "True"
            | "and"
            | "as"
            | "assert"
            | "async"
            | "await"
            | "break"
            | "class"
            | "continue"
            | "def"
            | "del"
            | "elif"
            | "else"
            | "except"
            | "finally"
            | "for"
            | "from"
            | "global"
            | "if"
            | "import"
            | "in"
            | "is"
            | "lambda"
            | "nonlocal"
            | "not"
            | "or"
            | "pass"
            | "raise"
            | "return"
            | "try"
            | "while"
            | "with"
            | "yield"
    ) {
        return false;
    }
    let mut bytes = name.bytes();
    matches!(bytes.next(), Some(b'a'..=b'z' | b'A'..=b'Z' | b'_'))
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

impl TextSignature {
    pub(crate) fn parse(text: String, span: Span) -> Result<Self> {
        let invalid = || Error::new(span, "invalid or unsupported text_signature override");
        let body = text
            .strip_prefix('(')
            .and_then(|s| s.strip_suffix(')'))
            .ok_or_else(invalid)?;
        if body.contains('\0') {
            return Err(invalid());
        }
        let mut params = Vec::new();
        let mut quote = None;
        let has_group = body.chars().any(|ch| {
            if matches!(ch, '\'' | '"') {
                quote = if quote == Some(ch) { None } else { Some(ch) };
            }
            ch == '[' && quote.is_none()
        });
        let omitted_slash = has_group && !body.ends_with(", /");
        if omitted_slash {
            // CPython's handwritten deque signatures predate '/' notation.
            // Brackets describe omission only; parameter kinds come from the binder.
            let mut groups = Vec::new();
            let mut token = String::new();
            let mut closed = false;
            let mut require_comma = false;
            let mut last_comma = false;
            for ch in body.chars().chain(['\0']) {
                if matches!(ch, '[' | ']' | ',' | '\0') {
                    let name = token.trim();
                    let had_name = !name.is_empty();
                    if closed && !matches!(ch, ']' | '\0') {
                        return Err(invalid());
                    }
                    if ch == ',' && !had_name && !require_comma {
                        return Err(invalid());
                    }
                    if had_name {
                        if closed {
                            return Err(invalid());
                        }
                        params.push(Parameter {
                            name: name.into(),
                            positional: false,
                            optional: !groups.is_empty(),
                            keywords: false,
                        });
                        token.clear();
                        last_comma = false;
                    }
                    match ch {
                        '[' => {
                            if closed {
                                return Err(invalid());
                            }
                            groups.push(params.len());
                            require_comma = had_name;
                            last_comma = false;
                        }
                        ']' => {
                            if last_comma {
                                return Err(invalid());
                            }
                            let start = groups.pop().ok_or_else(invalid)?;
                            if params.len() == start {
                                return Err(invalid());
                            }
                            closed = true;
                        }
                        ',' => {
                            require_comma = false;
                            last_comma = true;
                        }
                        _ => {
                            if last_comma || require_comma {
                                return Err(invalid());
                            }
                        }
                    }
                } else {
                    if require_comma && !ch.is_whitespace() {
                        return Err(invalid());
                    }
                    token.push(ch);
                }
            }
            if !groups.is_empty() || params.is_empty() {
                return Err(invalid());
            }
        } else if body.contains("[,") {
            let mut parts = body.split("[,").collect::<Vec<_>>();
            let optional_count = parts.len() - 1;
            let ending = format!("{}, /", "]".repeat(optional_count));
            let last = parts
                .pop()
                .unwrap()
                .strip_suffix(&ending)
                .ok_or_else(invalid)?;
            let required = parts.remove(0);
            for name in required.split(',').map(str::trim) {
                params.push(Parameter {
                    name: name.into(),
                    positional: true,
                    optional: false,
                    keywords: false,
                });
            }
            for name in parts.into_iter().chain([last]).map(str::trim) {
                params.push(Parameter {
                    name: name.into(),
                    positional: true,
                    optional: true,
                    keywords: false,
                });
            }
        } else {
            let mut slash = false;
            let mut optional = false;
            let mut keywords = false;
            for part in body.split(',').map(str::trim) {
                if keywords {
                    return Err(invalid());
                }
                if part == "/" {
                    if slash || params.is_empty() {
                        return Err(invalid());
                    }
                    slash = true;
                    for param in &mut params {
                        param.positional = true;
                    }
                    continue;
                }
                let (name, default) = match part.split_once('=') {
                    Some((name, default)) => {
                        let default = default.trim();
                        let quoted = (default.starts_with('\'') && default.ends_with('\''))
                            || (default.starts_with('"') && default.ends_with('"'));
                        let literal = if quoted && default.len() >= 2 {
                            !default[1..default.len() - 1].contains(['\'', '"', '\\', '\n', '\r'])
                        } else {
                            matches!(default, "None" | "True" | "False" | "<unrepresentable>")
                                || default
                                    .parse::<i128>()
                                    .is_ok_and(|value| value.to_string() == default)
                        };
                        if !literal {
                            return Err(invalid());
                        }
                        (name.trim(), true)
                    }
                    None => (part, false),
                };
                let name = if let Some(name) = name.strip_prefix("**") {
                    if default {
                        return Err(invalid());
                    }
                    keywords = true;
                    name
                } else {
                    name
                };
                if optional && !default && !keywords {
                    return Err(invalid());
                }
                optional |= default;
                params.push(Parameter {
                    name: name.into(),
                    positional: false,
                    optional: default,
                    keywords,
                });
            }
        }
        for (index, param) in params.iter().enumerate() {
            if !identifier(&param.name)
                || params[..index].iter().any(|other| other.name == param.name)
                || (param.name.starts_with('$')
                    && (index != 0
                        || (!param.positional && !omitted_slash)
                        || param.optional
                        || !matches!(param.name.as_str(), "$self" | "$type" | "$cls" | "$module")))
            {
                return Err(invalid());
            }
        }
        Ok(Self {
            text,
            params,
            omitted_slash,
        })
    }

    pub(crate) fn params_tokens(&self) -> Vec<TokenStream> {
        self.params.iter().map(|param| {
            let name = &param.name;
            let kind = if param.keywords { quote!(VarKeyword) } else if param.positional { quote!(PositionalOnly) } else { quote!(PositionalOrKeyword) };
            let default = if param.optional { quote!(Some(::rustpython_vm::function::DefaultRepr::Unrepresentable)) } else { quote!(None) };
            quote! { ::rustpython_vm::function::Param { name: #name, kind: ::rustpython_vm::function::ParamKind::#kind, default: #default } }
        }).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_class_and_handwritten_signatures() {
        for text in [
            "([iterable[, maxlen]])",
            "($self, value, [start, [stop]])",
            "(offset, name=<unrepresentable>)",
            "(name='[', other=None)",
            "($cls, name, bases, /, **kwds)",
        ] {
            assert!(
                TextSignature::parse(text.into(), Span::call_site()).is_ok(),
                "{text}"
            );
        }
        for text in [
            "([a]",
            "([])",
            "([a], b)",
            "(a, [b], c)",
            "(a, [b]])",
            "(a[b])",
            "([, a])",
            "([a,,b])",
            "([a],,,)",
            "([a,])",
        ] {
            assert!(
                TextSignature::parse(text.into(), Span::call_site()).is_err(),
                "{text}"
            );
        }
    }

    #[test]
    fn supported_overrides() {
        for name in ["sub", "prefix", "suffix"] {
            let sig = TextSignature::parse(
                format!("($self, {name}[, start[, end]], /)"),
                Span::call_site(),
            )
            .unwrap();
            assert_eq!(sig.params.len(), 4);
            assert!(sig.params[2..].iter().all(|p| p.positional && p.optional));
            assert!(!sig.params[1].optional);
        }
        for name in ["iterable", "fileobj", "name"] {
            let sig = TextSignature::parse(
                format!("($module, {name}, /, dialect='excel', **fmtparams)"),
                Span::call_site(),
            )
            .unwrap();
            assert_eq!(sig.params.len(), 4);
            assert!(sig.params[1].positional);
            assert!(!sig.params[2].positional);
            assert!(sig.params[2].optional);
            assert!(sig.params[3].keywords);
        }
    }

    #[test]
    fn malformed_overrides() {
        for text in [
            "()",
            "(class, /)",
            "(value=01, /)",
            "($self, sub[, ], /)",
            "($self, sub[, start[, ]], /)",
            "($self, sub[, start[, end], /)",
            "($self, $self, /)",
            "($module, name, /, **name)",
            "($module, name, /, **kwargs, **kwargs)",
            "($self, sub[, start[, end]]], /)",
            "($self, sub[, start], end, /)",
            "($self, sub[, start=0], /)",
            "($self, sub[, start], /, **kwargs)",
            "($self, sub[, start, end], /)",
            "($module, name, /, dialect=call(), **fmtparams)",
            "($module, name, /, dialect='excel', **fmtparams, extra)",
            "($module, name, /, /)",
            "($module, name, /, name=None)",
            "($module, name)",
            "(name, $self, /)",
            "($unknown, name, /)",
            "($module, name, /, dialect='excel', required)",
            "($module, name, /, **kwargs=None)",
            "($module, name, /, dialect='excel'junk')",
        ] {
            assert!(
                TextSignature::parse(text.into(), Span::call_site()).is_err(),
                "{text}"
            );
        }
    }
}
