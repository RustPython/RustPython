//! Parsing for Python macro arguments. Rust keywords such as `impl` are valid
//! option names, and literal arguments are retained for contextual diagnostics.
use proc_macro2::TokenStream;
use quote::ToTokens;
use syn::{
    Attribute, Expr, Ident, Lit, MacroDelimiter, Meta, Path, Result, Token,
    ext::IdentExt,
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
};

#[derive(Clone)]
pub enum NestedMeta {
    Meta(Meta),
    Lit(Lit),
}

pub(crate) type PunctuatedNestedMeta = Punctuated<NestedMeta, Token![,]>;

impl Parse for NestedMeta {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        if input.peek(Lit) && !(input.peek(syn::LitBool) && input.peek2(Token![=])) {
            return input.parse().map(Self::Lit);
        }
        if !input.peek(Ident::peek_any) && !input.peek(Token![::]) {
            return Err(input.error("expected identifier or literal"));
        }
        let leading_colon = input.parse()?;
        let segments = Punctuated::parse_separated_nonempty_with(input, |input| {
            Ident::parse_any(input).map(syn::PathSegment::from)
        })?;
        let path = Path {
            leading_colon,
            segments,
        };
        let meta = if input.peek(syn::token::Paren) {
            let content;
            let paren = syn::parenthesized!(content in input);
            let nested = content.parse_terminated(Self::parse, Token![,])?;
            Meta::List(syn::MetaList {
                path,
                delimiter: MacroDelimiter::Paren(paren),
                tokens: nested.into_token_stream(),
            })
        } else if input.peek(Token![=]) {
            let eq_token = input.parse()?;
            let mut value = input.parse()?;
            while let Expr::Group(group) = value {
                value = *group.expr;
            }
            Meta::NameValue(syn::MetaNameValue {
                path,
                eq_token,
                value,
            })
        } else {
            Meta::Path(path)
        };
        Ok(Self::Meta(meta))
    }
}

impl ToTokens for NestedMeta {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        match self {
            Self::Meta(meta) => meta.to_tokens(tokens),
            Self::Lit(lit) => lit.to_tokens(tokens),
        }
    }
}

impl NestedMeta {
    pub(crate) fn get_ident(&self) -> Option<&Ident> {
        match self {
            Self::Meta(meta) => meta.path().get_ident(),
            Self::Lit(_) => None,
        }
    }
}

pub(crate) fn nested_meta(list: &syn::MetaList) -> Result<PunctuatedNestedMeta> {
    if !matches!(list.delimiter, MacroDelimiter::Paren(_)) {
        return Err(syn::Error::new(
            list.delimiter.span().open(),
            "expected paren",
        ));
    }
    list.parse_args_with(PunctuatedNestedMeta::parse_terminated)
}

pub(crate) trait AttributeDoc {
    fn doc(self) -> Option<String>;
}

impl<'a, I: IntoIterator<Item = &'a Attribute>> AttributeDoc for I {
    fn doc(self) -> Option<String> {
        let docs: Vec<_> = self
            .into_iter()
            .filter_map(|attr| {
                let Meta::NameValue(meta) = &attr.meta else {
                    return None;
                };
                if !meta.path.is_ident("doc") {
                    return None;
                }
                let Expr::Lit(expr) = &meta.value else {
                    return None;
                };
                let Lit::Str(lit) = &expr.lit else {
                    return None;
                };
                Some(lit.value().trim().to_owned())
            })
            .collect();
        (!docs.is_empty()).then(|| docs.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;
    use syn::{parse::Parser, parse_quote};

    #[test]
    fn keyword_keys_and_nested_literals_round_trip() {
        let tokens = quote!(
            impl,
            module = false,
            with(::some::Trait),
            options("value", 7, true),
        );
        let args = PunctuatedNestedMeta::parse_terminated
            .parse2(tokens.clone())
            .unwrap();
        assert_eq!(args.to_token_stream().to_string(), tokens.to_string());
        assert_eq!(args.first().unwrap().get_ident().unwrap(), "impl");
        let NestedMeta::Meta(Meta::List(list)) = &args[3] else {
            panic!("expected list")
        };
        let nested = nested_meta(list).unwrap();
        assert_eq!(nested.len(), 3);
        assert!(nested.iter().all(|arg| matches!(arg, NestedMeta::Lit(_))));
    }

    #[test]
    fn boolean_name_value_is_not_a_literal() {
        let arg: NestedMeta = syn::parse_str("true = false").unwrap();
        assert!(matches!(arg, NestedMeta::Meta(Meta::NameValue(_))));
    }

    #[test]
    fn malformed_nested_arguments_are_rejected() {
        for source in ["with(a b)", "with(a,,)", "with(a::)", "name =", "with[a]"] {
            assert!(syn::parse_str::<NestedMeta>(source).is_err(), "{source}");
        }
        let list: syn::MetaList = parse_quote!(with[a]);
        assert_eq!(
            nested_meta(&list).err().unwrap().to_string(),
            "expected paren"
        );
    }

    #[test]
    fn docs_trim_each_line_and_ignore_non_strings() {
        let attrs: Vec<Attribute> = vec![
            parse_quote!(#[doc = " first "]),
            parse_quote!(#[cfg(test)]),
            parse_quote!(#[doc = 7]),
            parse_quote!(#[doc = " second "]),
        ];
        assert_eq!(attrs.doc().as_deref(), Some("first\nsecond"));
        let blank: Vec<Attribute> = vec![parse_quote!(#[doc = "  "])];
        assert_eq!(blank.doc().as_deref(), Some(""));
        assert_eq!(Vec::<Attribute>::new().doc(), None);
    }

    #[test]
    fn name_value_ungroups_macro_substitutions() {
        let mut tokens = quote!(name =);
        tokens.extend([proc_macro2::TokenTree::Group(proc_macro2::Group::new(
            proc_macro2::Delimiter::None,
            quote!("value"),
        ))]);
        let arg: NestedMeta = syn::parse2(tokens).unwrap();
        let NestedMeta::Meta(Meta::NameValue(value)) = arg else {
            panic!("expected name-value")
        };
        assert!(matches!(value.value, Expr::Lit(_)));
    }
}
