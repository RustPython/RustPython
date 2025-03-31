use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::ext::IdentExt;
use syn::meta::ParseNestedMeta;
use syn::{Attribute, Data, DeriveInput, Expr, Field, Ident, Result, Token, parse_quote};

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
    fn from_attribute(attr: &Attribute) -> Option<Result<ArgAttribute>> {
        if !attr.path().is_ident("pyarg") {
            return None;
        }
        let inner = move || {
            let mut arg_attr = None;
            attr.parse_nested_meta(|meta| {
                let Some(arg_attr) = &mut arg_attr else {
                    let kind = meta
                        .path
                        .get_ident()
                        .and_then(ParameterKind::from_ident)
                        .ok_or_else(|| {
                            meta.error(
                                "The first argument to #[pyarg()] must be the parameter type, \
                                 either 'positional', 'any', 'named', or 'flatten'.",
                            )
                        })?;
                    arg_attr = Some(ArgAttribute {
                        name: None,
                        kind,
                        default: None,
                    });
                    return Ok(());
                };
                arg_attr.parse_argument(meta)
            })?;
            arg_attr
                .ok_or_else(|| err_span!(attr, "There must be at least one argument to #[pyarg()]"))
        };
        Some(inner())
    }

    fn parse_argument(&mut self, meta: ParseNestedMeta<'_>) -> Result<()> {
        if let ParameterKind::Flatten = self.kind {
            return Err(meta.error("can't put additional arguments on a flatten arg"));
        }
        if meta.path.is_ident("default") && meta.input.peek(Token![=]) {
            if matches!(self.default, Some(Some(_))) {
                return Err(meta.error("Default already set"));
            }
            let val = meta.value()?;
            self.default = Some(Some(val.parse()?))
        } else if meta.path.is_ident("default") || meta.path.is_ident("optional") {
            if self.default.is_none() {
                self.default = Some(None);
            }
        } else if meta.path.is_ident("name") {
            if self.name.is_some() {
                return Err(meta.error("already have a name"));
            }
            let val = meta.value()?.parse::<syn::LitStr>()?;
            self.name = Some(val.value())
        } else {
            return Err(meta.error("Unrecognized pyarg attribute"));
        }

        Ok(())
    }
}

fn generate_field((i, field): (usize, &Field)) -> Result<TokenStream> {
    let mut pyarg_attrs = field
        .attrs
        .iter()
        .filter_map(ArgAttribute::from_attribute)
        .collect::<std::result::Result<Vec<_>, _>>()?;
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
    let name_string = name.map(|ident| ident.unraw().to_string());
    if matches!(&name_string, Some(s) if s.starts_with("_phantom")) {
        return Ok(quote! {
            #name: ::std::marker::PhantomData,
        });
    }
    let field_name = match name {
        Some(id) => id.to_token_stream(),
        None => syn::Index::from(i).into_token_stream(),
    };
    if let ParameterKind::Flatten = attr.kind {
        return Ok(quote! {
            #field_name: ::rustpython_vm::function::FromArgs::from_args(vm, args)?,
        });
    }
    let pyname = attr
        .name
        .or(name_string)
        .ok_or_else(|| err_span!(field, "field in tuple struct must have name attribute"))?;
    let middle = quote! {
        .map(|x| ::rustpython_vm::convert::TryFromObject::try_from_object(vm, x)).transpose()?
    };
    let ending = if let Some(default) = attr.default {
        let ty = &field.ty;
        let default = default.unwrap_or_else(|| parse_quote!(::std::default::Default::default()));
        quote! {
            .map(<#ty as ::rustpython_vm::function::FromArgOptional>::from_inner)
            .unwrap_or_else(|| #default)
        }
    } else {
        let err = match attr.kind {
            ParameterKind::PositionalOnly | ParameterKind::PositionalOrKeyword => quote! {
                ::rustpython_vm::function::ArgumentError::TooFewArgs
            },
            ParameterKind::KeywordOnly => quote! {
                ::rustpython_vm::function::ArgumentError::RequiredKeywordArgument(#pyname.to_owned())
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
                #field_name: args.take_positional()#middle #ending,
            }
        }
        ParameterKind::PositionalOrKeyword => {
            quote! {
                #field_name: args.take_positional_keyword(#pyname)#middle #ending,
            }
        }
        ParameterKind::KeywordOnly => {
            quote! {
                #field_name: args.take_keyword(#pyname)#middle #ending,
            }
        }
        ParameterKind::Flatten => unreachable!(),
    };
    Ok(file_output)
}

pub fn impl_from_args(input: DeriveInput) -> Result<TokenStream> {
    let fields = match input.data {
        Data::Struct(syn::DataStruct { fields, .. }) => fields
            .iter()
            .enumerate()
            .map(generate_field)
            .collect::<Result<TokenStream>>()?,
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
