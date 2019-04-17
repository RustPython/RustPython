use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Attribute, Data, DeriveInput, Expr, Field, Fields, Ident, Lit, Meta, NestedMeta};

/// The kind of the python parameter, this corresponds to the value of Parameter.kind
/// (https://docs.python.org/3/library/inspect.html#inspect.Parameter.kind)
enum ParameterKind {
    PositionalOnly,
    PositionalOrKeyword,
    KeywordOnly,
}

impl ParameterKind {
    fn from_ident(ident: &Ident) -> ParameterKind {
        if ident == "positional_only" {
            ParameterKind::PositionalOnly
        } else if ident == "positional_or_keyword" {
            ParameterKind::PositionalOrKeyword
        } else if ident == "keyword_only" {
            ParameterKind::KeywordOnly
        } else {
            panic!("Unrecognised attribute")
        }
    }
}

struct ArgAttribute {
    kind: ParameterKind,
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
                    NestedMeta::Meta(Meta::Word(ident)) => ParameterKind::from_ident(ident),
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
            kind: ParameterKind::PositionalOrKeyword,
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
        .map(|x| ::rustpython_vm::pyobject::TryFromObject::try_from_object(vm, x)).transpose()?
    };
    let ending = if let Some(default) = attr.default {
        quote! {
            .unwrap_or_else(|| #default)
        }
    } else if attr.optional {
        quote! {
            .map(::rustpython_vm::function::OptionalArg::Present)
            .unwrap_or(::rustpython_vm::function::OptionalArg::Missing)
        }
    } else {
        let err = match attr.kind {
            ParameterKind::PositionalOnly | ParameterKind::PositionalOrKeyword => quote! {
                ::rustpython_vm::function::ArgumentError::TooFewArgs
            },
            ParameterKind::KeywordOnly => quote! {
                ::rustpython_vm::function::ArgumentError::RequiredKeywordArgument(tringify!(#name))
            },
        };
        quote! {
            .ok_or_else(|| #err)?
        }
    };

    match attr.kind {
        ParameterKind::PositionalOnly => {
            quote! {
                #name: args.take_positional()#middle#ending,
            }
        }
        ParameterKind::PositionalOrKeyword => {
            quote! {
                #name: args.take_positional_keyword(stringify!(#name))#middle#ending,
            }
        }
        ParameterKind::KeywordOnly => {
            quote! {
                #name: args.take_keyword(stringify!(#name))#middle#ending,
            }
        }
    }
}

pub fn impl_from_args(input: DeriveInput) -> TokenStream2 {
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
        impl ::rustpython_vm::function::FromArgs for #name {
            fn from_args(
                vm: &::rustpython_vm::VirtualMachine,
                args: &mut ::rustpython_vm::function::PyFuncArgs
            ) -> Result<Self, ::rustpython_vm::function::ArgumentError> {
                Ok(#name { #(#fields)* })
            }
        }
    }
}
