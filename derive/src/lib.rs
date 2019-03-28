extern crate proc_macro;

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Attribute, Data, DeriveInput, Expr, Field, Fields, Ident, Lit, Meta, NestedMeta};

#[proc_macro_derive(FromArgs, attributes(pyarg))]
pub fn derive_from_args(input: TokenStream) -> TokenStream {
    let ast: DeriveInput = syn::parse(input).unwrap();

    let gen = impl_from_args(&ast);
    gen.to_string().parse().unwrap()
}

enum ArgKind {
    Positional,
    PositionalKeyword,
    Keyword,
}

impl ArgKind {
    fn from_ident(ident: &Ident) -> ArgKind {
        if ident == "positional" {
            ArgKind::Positional
        } else if ident == "positional_keyword" {
            ArgKind::PositionalKeyword
        } else if ident == "keyword" {
            ArgKind::Keyword
        } else {
            panic!("Unrecognised attribute")
        }
    }
}

struct ArgAttribute {
    kind: ArgKind,
    default: Option<Expr>,
    optional: bool,
}

impl ArgAttribute {
    fn from_attribute(attr: &Attribute) -> Option<ArgAttribute> {
        if !attr.path.is_ident("pyarg") {
            return None;
        }

        match attr.parse_meta().unwrap() {
            Meta::List(list) => {
                let mut iter = list.nested.iter();
                let first_arg = iter.next().expect("at least one argument in pyarg list");
                let kind = match first_arg {
                    NestedMeta::Meta(Meta::Word(ident)) => ArgKind::from_ident(ident),
                    _ => panic!("Bad syntax for first pyarg attribute argument"),
                };

                let mut attribute = ArgAttribute {
                    kind,
                    default: None,
                    optional: false,
                };

                while let Some(arg) = iter.next() {
                    attribute.parse_argument(arg);
                }

                assert!(
                    attribute.default.is_none() || !attribute.optional,
                    "Can't set both a default value and optional"
                );

                Some(attribute)
            }
            _ => panic!("Bad syntax for pyarg attribute"),
        }
    }

    fn parse_argument(&mut self, arg: &NestedMeta) {
        match arg {
            NestedMeta::Meta(Meta::Word(ident)) => {
                if ident == "default" {
                    assert!(self.default.is_none(), "Default already set");
                    let expr = syn::parse_str::<Expr>("Default::default()").unwrap();
                    self.default = Some(expr);
                } else if ident == "optional" {
                    self.optional = true;
                } else {
                    panic!("Unrecognised pyarg attribute '{}'", ident);
                }
            }
            NestedMeta::Meta(Meta::NameValue(name_value)) => {
                if name_value.ident == "default" {
                    assert!(self.default.is_none(), "Default already set");

                    match name_value.lit {
                        Lit::Str(ref val) => {
                            let expr = val
                                .parse::<Expr>()
                                .expect("a valid expression for default argument");
                            self.default = Some(expr);
                        }
                        _ => panic!("Expected string value for default argument"),
                    }
                } else if name_value.ident == "optional" {
                    match name_value.lit {
                        Lit::Bool(ref val) => {
                            self.optional = val.value;
                        }
                        _ => panic!("Expected boolean value for optional argument"),
                    }
                } else {
                    panic!("Unrecognised pyarg attribute '{}'", name_value.ident);
                }
            }
            _ => panic!("Bad syntax for first pyarg attribute argument"),
        };
    }
}

fn generate_field(field: &Field) -> TokenStream2 {
    let mut pyarg_attrs = field
        .attrs
        .iter()
        .filter_map(ArgAttribute::from_attribute)
        .collect::<Vec<_>>();
    let attr = if pyarg_attrs.is_empty() {
        ArgAttribute {
            kind: ArgKind::PositionalKeyword,
            default: None,
            optional: false,
        }
    } else if pyarg_attrs.len() == 1 {
        pyarg_attrs.remove(0)
    } else {
        panic!(
            "Multiple pyarg attributes on field '{}'",
            field.ident.as_ref().unwrap()
        );
    };

    let name = &field.ident;
    let middle = quote! {
        .map(|x| crate::pyobject::TryFromObject::try_from_object(vm, x)).transpose()?
    };
    let ending = if let Some(default) = attr.default {
        quote! {
            .unwrap_or_else(|| #default)
        }
    } else if attr.optional {
        quote! {
            .map(crate::function::OptionalArg::Present)
            .unwrap_or(crate::function::OptionalArg::Missing)
        }
    } else {
        let err = match attr.kind {
            ArgKind::Positional | ArgKind::PositionalKeyword => quote! {
                crate::function::ArgumentError::TooFewArgs
            },
            ArgKind::Keyword => quote! {
                crate::function::ArgumentError::RequiredKeywordArgument(tringify!(#name))
            },
        };
        quote! {
            .ok_or_else(|| #err)?
        }
    };

    match attr.kind {
        ArgKind::Positional => {
            quote! {
                #name: args.take_positional()#middle#ending,
            }
        }
        ArgKind::PositionalKeyword => {
            quote! {
                #name: args.take_positional_keyword(stringify!(#name))#middle#ending,
            }
        }
        ArgKind::Keyword => {
            quote! {
                #name: args.take_keyword(stringify!(#name))#middle#ending,
            }
        }
    }
}

fn impl_from_args(input: &DeriveInput) -> TokenStream2 {
    // FIXME: This references types using `crate` instead of `rustpython_vm`
    //        so that it can be used in the latter. How can we support both?
    //        Can use extern crate self as rustpython_vm; once in stable.
    //        https://github.com/rust-lang/rust/issues/56409
    let fields = match input.data {
        Data::Struct(ref data) => {
            match data.fields {
                Fields::Named(ref fields) => fields.named.iter().map(generate_field),
                Fields::Unnamed(_) | Fields::Unit => unimplemented!(), // TODO: better error message
            }
        }
        Data::Enum(_) | Data::Union(_) => unimplemented!(), // TODO: better error message
    };

    let name = &input.ident;
    quote! {
        impl crate::function::FromArgs for #name {
            fn from_args(
                vm: &crate::vm::VirtualMachine,
                args: &mut crate::function::PyFuncArgs
            ) -> Result<Self, crate::function::ArgumentError> {
                Ok(#name { #(#fields)* })
            }
        }
    }
}
