use crate::util::path_eq;
use crate::Diagnostic;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{
    parse_quote, Attribute, Data, DeriveInput, Expr, Field, Fields, Ident, Lit, Meta, NestedMeta,
};

/// The kind of the python parameter, this corresponds to the value of Parameter.kind
/// (https://docs.python.org/3/library/inspect.html#inspect.Parameter.kind)
enum ParameterKind {
    PositionalOnly,
    PositionalOrKeyword,
    KeywordOnly,
}

impl ParameterKind {
    fn from_ident(ident: &Ident) -> Option<ParameterKind> {
        match ident.to_string().as_str() {
            "positional_only" => Some(ParameterKind::PositionalOnly),
            "positional_or_keyword" => Some(ParameterKind::PositionalOrKeyword),
            "keyword_only" => Some(ParameterKind::KeywordOnly),
            _ => None,
        }
    }
}

struct ArgAttribute {
    kind: ParameterKind,
    default: Option<Expr>,
    optional: bool,
}

impl ArgAttribute {
    fn from_attribute(attr: &Attribute) -> Option<Result<ArgAttribute, Diagnostic>> {
        if !attr.path.is_ident("pyarg") {
            return None;
        }
        let inner = move || match attr.parse_meta()? {
            Meta::List(list) => {
                let mut iter = list.nested.iter();
                let first_arg = iter.next().ok_or_else(|| {
                    err_span!(list, "There must be at least one argument to #[pyarg()]")
                })?;
                let kind = match first_arg {
                    NestedMeta::Meta(Meta::Path(path)) => {
                        path.get_ident().and_then(ParameterKind::from_ident)
                    }
                    _ => None,
                };
                let kind = kind.ok_or_else(|| {
                    err_span!(
                        first_arg,
                        "The first argument to #[pyarg()] must be the parameter type, either \
                         'positional_only', 'positional_or_keyword', or 'keyword_only'."
                    )
                })?;

                let mut attribute = ArgAttribute {
                    kind,
                    default: None,
                    optional: false,
                };

                for arg in iter {
                    attribute.parse_argument(arg)?;
                }

                if attribute.default.is_some() && attribute.optional {
                    bail_span!(attr, "Can't set both a default value and optional");
                }

                Ok(attribute)
            }
            _ => bail_span!(attr, "pyarg must be a list, like #[pyarg(...)]"),
        };
        Some(inner())
    }

    fn parse_argument(&mut self, arg: &NestedMeta) -> Result<(), Diagnostic> {
        match arg {
            NestedMeta::Meta(Meta::Path(path)) => {
                if path_eq(&path, "default") {
                    if self.default.is_some() {
                        bail_span!(path, "Default already set");
                    }
                    let expr = parse_quote!(Default::default());
                    self.default = Some(expr);
                } else if path_eq(&path, "optional") {
                    self.optional = true;
                } else {
                    bail_span!(path, "Unrecognised pyarg attribute");
                }
            }
            NestedMeta::Meta(Meta::NameValue(name_value)) => {
                if path_eq(&name_value.path, "default") {
                    if self.default.is_some() {
                        bail_span!(name_value, "Default already set");
                    }

                    match name_value.lit {
                        Lit::Str(ref val) => {
                            let expr = val.parse::<Expr>().map_err(|_| {
                                err_span!(val, "Expected a valid expression for default argument")
                            })?;
                            self.default = Some(expr);
                        }
                        _ => bail_span!(name_value, "Expected string value for default argument"),
                    }
                } else if path_eq(&name_value.path, "optional") {
                    match name_value.lit {
                        Lit::Bool(ref val) => {
                            self.optional = val.value;
                        }
                        _ => bail_span!(
                            name_value.lit,
                            "Expected boolean value for optional argument"
                        ),
                    }
                } else {
                    bail_span!(name_value, "Unrecognised pyarg attribute");
                }
            }
            _ => bail_span!(arg, "Unrecognised pyarg attribute"),
        }

        Ok(())
    }
}

fn generate_field(field: &Field) -> Result<TokenStream2, Diagnostic> {
    let mut pyarg_attrs = field
        .attrs
        .iter()
        .filter_map(ArgAttribute::from_attribute)
        .collect::<Result<Vec<_>, _>>()?;
    let attr = if pyarg_attrs.is_empty() {
        ArgAttribute {
            kind: ParameterKind::PositionalOrKeyword,
            default: None,
            optional: false,
        }
    } else if pyarg_attrs.len() == 1 {
        pyarg_attrs.remove(0)
    } else {
        bail_span!(field, "Multiple pyarg attributes on field");
    };

    let name = &field.ident;
    if let Some(name) = name {
        if name.to_string().starts_with("_phantom") {
            return Ok(quote! {
                #name: std::marker::PhantomData,
            });
        }
    }
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

    let file_output = match attr.kind {
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
    };
    Ok(file_output)
}

pub fn impl_from_args(input: DeriveInput) -> Result<TokenStream2, Diagnostic> {
    let fields = match input.data {
        Data::Struct(syn::DataStruct {
            fields: Fields::Named(fields),
            ..
        }) => fields
            .named
            .iter()
            .map(generate_field)
            .collect::<Result<TokenStream2, Diagnostic>>()?,
        _ => bail_span!(input, "FromArgs input must be a struct with named fields"),
    };

    let name = input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    let output = quote! {
        impl #impl_generics ::rustpython_vm::function::FromArgs for #name #ty_generics #where_clause {
            fn from_args(
                vm: &::rustpython_vm::VirtualMachine,
                args: &mut ::rustpython_vm::function::PyFuncArgs
            ) -> Result<Self, ::rustpython_vm::function::ArgumentError> {
                Ok(#name { #fields })
            }
        }
    };
    Ok(output)
}
