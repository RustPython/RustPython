use crate::util::path_eq;
use crate::Diagnostic;
use proc_macro2::TokenStream as TokenStream2;
use quote::{quote, ToTokens};
use syn::{parse_quote, Attribute, Data, DeriveInput, Expr, Field, Ident, Lit, Meta, NestedMeta};

/// The kind of the python parameter, this corresponds to the value of Parameter.kind
/// (https://docs.python.org/3/library/inspect.html#inspect.Parameter.kind)
enum ParameterKind {
    PositionalOnly,
    PositionalOrKeyword,
    KeywordOnly,
    Flatten,
}

impl ParameterKind {
    fn from_ident(ident: &Ident) -> Option<ParameterKind> {
        match ident.to_string().as_str() {
            "positional" => Some(ParameterKind::PositionalOnly),
            "any" => Some(ParameterKind::PositionalOrKeyword),
            "named" => Some(ParameterKind::KeywordOnly),
            "flatten" => Some(ParameterKind::Flatten),
            _ => None,
        }
    }
}

struct ArgAttribute {
    name: Option<String>,
    kind: ParameterKind,
    default: Option<DefaultValue>,
}
// None == quote!(Default::default())
type DefaultValue = Option<Expr>;

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
                         'positional', 'any', 'named', or 'flatten'."
                    )
                })?;

                let mut attribute = ArgAttribute {
                    name: None,
                    kind,
                    default: None,
                };

                for arg in iter {
                    attribute.parse_argument(arg)?;
                }

                Ok(attribute)
            }
            _ => bail_span!(attr, "pyarg must be a list, like #[pyarg(...)]"),
        };
        Some(inner())
    }

    fn parse_argument(&mut self, arg: &NestedMeta) -> Result<(), Diagnostic> {
        if let ParameterKind::Flatten = self.kind {
            bail_span!(arg, "can't put additional arguments on a flatten arg")
        }
        match arg {
            NestedMeta::Meta(Meta::Path(path)) => {
                if path_eq(&path, "default") || path_eq(&path, "optional") {
                    if self.default.is_none() {
                        self.default = Some(None);
                    }
                } else {
                    bail_span!(path, "Unrecognised pyarg attribute");
                }
            }
            NestedMeta::Meta(Meta::NameValue(name_value)) => {
                if path_eq(&name_value.path, "default") {
                    if matches!(self.default, Some(Some(_))) {
                        bail_span!(name_value, "Default already set");
                    }

                    match name_value.lit {
                        Lit::Str(ref val) => self.default = Some(Some(val.parse()?)),
                        _ => bail_span!(name_value, "Expected string value for default argument"),
                    }
                } else if path_eq(&name_value.path, "name") {
                    if self.name.is_some() {
                        bail_span!(name_value, "already have a name")
                    }

                    match &name_value.lit {
                        Lit::Str(val) => self.name = Some(val.value()),
                        _ => bail_span!(name_value, "Expected string value for name argument"),
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

fn generate_field((i, field): (usize, &Field)) -> Result<TokenStream2, Diagnostic> {
    let mut pyarg_attrs = field
        .attrs
        .iter()
        .filter_map(ArgAttribute::from_attribute)
        .collect::<Result<Vec<_>, _>>()?;
    let attr = if pyarg_attrs.is_empty() {
        ArgAttribute {
            name: None,
            kind: ParameterKind::PositionalOrKeyword,
            default: None,
        }
    } else if pyarg_attrs.len() == 1 {
        pyarg_attrs.remove(0)
    } else {
        bail_span!(field, "Multiple pyarg attributes on field");
    };

    let name = field.ident.as_ref();
    let namestring = name.map(Ident::to_string);
    if matches!(&namestring, Some(s) if s.starts_with("_phantom")) {
        return Ok(quote! {
            #name: ::std::marker::PhantomData,
        });
    }
    let fieldname = match name {
        Some(id) => id.to_token_stream(),
        None => syn::Index::from(i).into_token_stream(),
    };
    if let ParameterKind::Flatten = attr.kind {
        return Ok(quote! {
            #fieldname: ::rustpython_vm::function::FromArgs::from_args(vm, args)?,
        });
    }
    let pyname = attr
        .name
        .or(namestring)
        .ok_or_else(|| err_span!(field, "field in tuple struct must have name attribute"))?;
    let middle = quote! {
        .map(|x| ::rustpython_vm::pyobject::TryFromObject::try_from_object(vm, x)).transpose()?
    };
    let ending = if let Some(default) = attr.default {
        let default = default.unwrap_or_else(|| parse_quote!(::std::default::Default::default()));
        quote! {
            .map(::rustpython_vm::function::FromArgOptional::from_inner)
            .unwrap_or_else(|| #default)
        }
    } else {
        let err = match attr.kind {
            ParameterKind::PositionalOnly | ParameterKind::PositionalOrKeyword => quote! {
                ::rustpython_vm::function::ArgumentError::TooFewArgs
            },
            ParameterKind::KeywordOnly => quote! {
                ::rustpython_vm::function::ArgumentError::RequiredKeywordArgument(#pyname)
            },
            ParameterKind::Flatten => unreachable!(),
        };
        quote! {
            .ok_or_else(|| #err)?
        }
    };

    let file_output = match attr.kind {
        ParameterKind::PositionalOnly => {
            quote! {
                #fieldname: args.take_positional()#middle#ending,
            }
        }
        ParameterKind::PositionalOrKeyword => {
            quote! {
                #fieldname: args.take_positional_keyword(#pyname)#middle#ending,
            }
        }
        ParameterKind::KeywordOnly => {
            quote! {
                #fieldname: args.take_keyword(#pyname)#middle#ending,
            }
        }
        ParameterKind::Flatten => unreachable!(),
    };
    Ok(file_output)
}

pub fn impl_from_args(input: DeriveInput) -> Result<TokenStream2, Diagnostic> {
    let fields = match input.data {
        Data::Struct(syn::DataStruct { fields, .. }) => fields
            .iter()
            .enumerate()
            .map(generate_field)
            .collect::<Result<TokenStream2, Diagnostic>>()?,
        _ => bail_span!(input, "FromArgs input must be a struct"),
    };

    let name = input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    let output = quote! {
        impl #impl_generics ::rustpython_vm::function::FromArgs for #name #ty_generics #where_clause {
            fn from_args(
                vm: &::rustpython_vm::VirtualMachine,
                args: &mut ::rustpython_vm::function::FuncArgs
            ) -> ::std::result::Result<Self, ::rustpython_vm::function::ArgumentError> {
                Ok(#name { #fields })
            }
        }
    };
    Ok(output)
}
