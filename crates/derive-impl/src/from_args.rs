use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::ext::IdentExt;
use syn::meta::ParseNestedMeta;
use syn::{
    Attribute, Data, DeriveInput, Expr, Field, Ident, Lit, Result, Token, Type, parse_quote,
};

/// The kind of the python parameter, this corresponds to the value of Parameter.kind
/// (https://docs.python.org/3/library/inspect.html#inspect.Parameter.kind)
#[derive(Clone, Copy, Default, Eq, PartialEq)]
enum ParameterKind {
    PositionalOnly,
    #[default]
    PositionalOrKeyword,
    KeywordOnly,
    Flatten,
}

impl TryFrom<&Ident> for ParameterKind {
    type Error = ();

    fn try_from(ident: &Ident) -> core::result::Result<Self, Self::Error> {
        Ok(match ident.to_string().as_str() {
            "positional" => Self::PositionalOnly,
            "any" => Self::PositionalOrKeyword,
            "named" => Self::KeywordOnly,
            "flatten" => Self::Flatten,
            _ => return Err(()),
        })
    }
}

// None == quote!(Default::default())
type DefaultValue = Option<Expr>;

#[derive(Default)]
struct ArgAttribute {
    name: Option<String>,
    kind: ParameterKind,
    default: Option<DefaultValue>,
    /// `optional`: missing argument is `Default::default()`. The signature default
    /// comes from `OptionalArgDefault` on the field type. Bare `default` is separate.
    optional: bool,
    py_default: Option<String>,
    error_msg: Option<String>,
}

impl ArgAttribute {
    fn from_attribute(attr: &Attribute) -> Option<Result<Self>> {
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
                        .and_then(|ident| ParameterKind::try_from(ident).ok())
                        .ok_or_else(|| {
                            meta.error(
                                "The first argument to #[pyarg()] must be the parameter type, \
                                 either 'positional', 'any', 'named', or 'flatten'.",
                            )
                        })?;
                    arg_attr = Some(Self {
                        name: None,
                        kind,
                        default: None,
                        optional: false,
                        py_default: None,
                        error_msg: None,
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
        if self.kind == ParameterKind::Flatten {
            return Err(meta.error("can't put additional arguments on a flatten arg"));
        }

        if meta.path.is_ident("default") && meta.input.peek(Token![=]) {
            if matches!(self.default, Some(Some(_))) {
                return Err(meta.error("Default already set"));
            }
            let val = meta.value()?;
            self.default = Some(Some(val.parse()?))
        } else if meta.path.is_ident("optional") {
            self.optional = true;
            if self.default.is_none() {
                self.default = Some(None);
            }
        } else if meta.path.is_ident("default") {
            if self.default.is_none() {
                self.default = Some(None);
            }
        // Text-only signature default. See the `FromArgs` derive docs.
        } else if meta.path.is_ident("py_default") {
            if self.py_default.is_some() {
                return Err(meta.error("py_default already set"));
            }
            let val = meta.value()?.parse::<syn::LitStr>()?;
            if val.value() == "<unrepresentable>" {
                return Err(meta.error(
                    "py_default = \"<unrepresentable>\" is not allowed; use OptionalArg when a \
                     missing argument is a distinct state, or give the default's type a real \
                     py_default()",
                ));
            }
            self.py_default = Some(val.value());
        } else if meta.path.is_ident("name") {
            if self.name.is_some() {
                return Err(meta.error("already have a name"));
            }
            let val = meta.value()?.parse::<syn::LitStr>()?;
            self.name = Some(val.value())
        } else if meta.path.is_ident("error_msg") {
            if self.error_msg.is_some() {
                return Err(meta.error("already have an error_msg"));
            }
            let val = meta.value()?.parse::<syn::LitStr>()?;
            self.error_msg = Some(val.value())
        } else {
            return Err(meta.error("Unrecognized pyarg attribute"));
        }

        Ok(())
    }
}

impl TryFrom<&Field> for ArgAttribute {
    type Error = syn::Error;

    fn try_from(field: &Field) -> core::result::Result<Self, Self::Error> {
        let mut pyarg_attrs = field
            .attrs
            .iter()
            .filter_map(Self::from_attribute)
            .collect::<core::result::Result<Vec<_>, _>>()?;

        if pyarg_attrs.len() >= 2 {
            bail_span!(field, "Multiple pyarg attributes on field")
        };

        Ok(pyarg_attrs.pop().unwrap_or_default())
    }
}

fn generate_field((i, field): (usize, &Field)) -> Result<TokenStream> {
    let attr = ArgAttribute::try_from(field)?;
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

    let middle = if let Some(error_msg) = &attr.error_msg {
        quote! {
            .map(|x| ::rustpython_vm::convert::TryFromObject::try_from_object(vm, x)
                .map_err(|_| vm.new_type_error(#error_msg))).transpose()?
        }
    } else {
        quote! {
            .map(|x| ::rustpython_vm::convert::TryFromObject::try_from_object(vm, x)).transpose()?
        }
    };

    let ending = if let Some(default) = attr.default {
        let ty = &field.ty;
        let literal_obj = match &default {
            Some(expr) if !keeps_direct_literal(ty) && !is_wrapped_arg_ty(ty) => {
                literal_object_expr(expr)
            }
            _ => None,
        };
        if let Some(obj) = literal_obj {
            quote! {
                .map(<#ty as ::rustpython_vm::function::FromArgOptional>::from_inner)
                .map(::core::result::Result::Ok)
                .unwrap_or_else(|| {
                    <#ty as ::rustpython_vm::convert::TryFromObject>::try_from_object(
                        vm,
                        ::rustpython_vm::convert::ToPyObject::to_pyobject(#obj, vm),
                    )
                    .map_err(::rustpython_vm::function::ArgumentError::from)
                })?
            }
        } else {
            let default = match default {
                Some(expr) => match absolute_const_ident(&expr)? {
                    Some(name) => parse_quote!(::core::convert::Into::into(#name)),
                    None => expr,
                },
                None => parse_quote!(::std::default::Default::default()),
            };
            quote! {
                .map(<#ty as ::rustpython_vm::function::FromArgOptional>::from_inner)
                .unwrap_or_else(|| #default)
            }
        }
    } else {
        // A parameter a call may name is named back when it is missing; one
        // that can only be passed by position is only ever counted.
        let err = match attr.kind {
            ParameterKind::PositionalOnly => quote! {
                ::rustpython_vm::function::ArgumentError::TooFewArgs
            },
            ParameterKind::PositionalOrKeyword | ParameterKind::KeywordOnly => {
                let pos = i + 1;
                quote! {
                    ::rustpython_vm::function::ArgumentError::MissingRequiredArgument {
                        name: #pyname.to_owned(),
                        pos: #pos,
                    }
                }
            }
            ParameterKind::Flatten => unreachable!(),
        };
        quote! {
            .ok_or_else(|| #err)?
        }
    };

    let file_output = match attr.kind {
        ParameterKind::PositionalOnly => quote! {
            #field_name: args.take_positional()#middle #ending,
        },
        ParameterKind::PositionalOrKeyword => quote! {
            #field_name: args.take_positional_keyword(#pyname)#middle #ending,
        },
        ParameterKind::KeywordOnly => quote! {
            #field_name: args.take_keyword(#pyname)#middle #ending,
        },
        ParameterKind::Flatten => unreachable!(),
    };

    Ok(file_output)
}

fn is_phantom(field: &Field) -> bool {
    field
        .ident
        .as_ref()
        .is_some_and(|ident| ident.unraw().to_string().starts_with("_phantom"))
}

fn repr_path() -> TokenStream {
    quote!(::rustpython_vm::function::DefaultRepr)
}

fn is_primitive_int(ty: &Type) -> bool {
    let Type::Path(path) = ty else {
        return false;
    };
    let Some(last) = path.path.segments.last() else {
        return false;
    };
    // The last segment decides, so `core::ffi::c_int` counts too.
    path.qself.is_none()
        && matches!(last.arguments, syn::PathArguments::None)
        && matches!(
            last.ident.to_string().as_str(),
            "i8" | "i16"
                | "i32"
                | "i64"
                | "i128"
                | "isize"
                | "u8"
                | "u16"
                | "u32"
                | "u64"
                | "u128"
                | "usize"
                | "c_short"
                | "c_ushort"
                | "c_int"
                | "c_uint"
                | "c_long"
                | "c_ulong"
                | "c_longlong"
                | "c_ulonglong"
        )
}

/// `default = ::NAME` copies `NAME` into the signature. The Rust value is
/// `Into::into(NAME)`: a leading `::` would name an extern crate.
fn absolute_const_ident(expr: &Expr) -> Result<Option<Ident>> {
    let Expr::Path(path) = expr else {
        return Ok(None);
    };
    if path.qself.is_some() || path.path.leading_colon.is_none() {
        return Ok(None);
    }
    let segments = &path.path.segments;
    if segments.len() == 1 && matches!(segments[0].arguments, syn::PathArguments::None) {
        return Ok(Some(segments[0].ident.clone()));
    }
    bail_span!(
        expr,
        "`default = ::NAME` takes one identifier so that name is copied into the signature; \
         use a plain path for a typed value"
    )
}

fn is_bool_ty(ty: &Type) -> bool {
    matches!(&ty, Type::Path(path) if path.qself.is_none() && path.path.is_ident("bool"))
}

fn is_float_ty(ty: &Type) -> bool {
    let Type::Path(path) = ty else {
        return false;
    };
    let Some(last) = path.path.segments.last() else {
        return false;
    };
    path.qself.is_none()
        && matches!(last.arguments, syn::PathArguments::None)
        && matches!(last.ident.to_string().as_str(), "f32" | "f64")
}

fn is_static_str(ty: &Type) -> bool {
    let Type::Reference(reference) = ty else {
        return false;
    };
    reference
        .lifetime
        .as_ref()
        .is_some_and(|lifetime| lifetime.ident == "static")
        && matches!(
            reference.elem.as_ref(),
            Type::Path(path) if path.qself.is_none() && path.path.is_ident("str")
        )
}

/// Primitives and `&'static str` store the literal itself.
fn keeps_direct_literal(ty: &Type) -> bool {
    is_primitive_int(ty) || is_bool_ty(ty) || is_float_ty(ty) || is_static_str(ty)
}

/// `optional` / `OptionalArg` / `Option` keep their own missing-state rules.
fn is_wrapped_arg_ty(ty: &Type) -> bool {
    let Type::Path(path) = ty else {
        return false;
    };
    let Some(last) = path.path.segments.last() else {
        return false;
    };
    matches!(
        last.ident.to_string().as_str(),
        "OptionalArg" | "Option" | "OptionalOption"
    )
}

/// Expression passed to `ToPyObject` for a string, bytes, int, float, or bool literal.
fn literal_object_expr(expr: &Expr) -> Option<TokenStream> {
    match expr {
        Expr::Lit(syn::ExprLit { lit, .. }) => match lit {
            Lit::Str(_) | Lit::ByteStr(_) | Lit::Bool(_) => Some(expr.to_token_stream()),
            Lit::Int(_) => Some(quote!((#expr as i128))),
            Lit::Float(_) => Some(quote!((#expr as f64))),
            _ => None,
        },
        Expr::Unary(syn::ExprUnary {
            op: syn::UnOp::Neg(_),
            expr: inner,
            ..
        }) => match inner.as_ref() {
            Expr::Lit(syn::ExprLit {
                lit: Lit::Int(_), ..
            }) => Some(quote!((#expr as i128))),
            Expr::Lit(syn::ExprLit {
                lit: Lit::Float(_), ..
            }) => Some(quote!((#expr as f64))),
            _ => None,
        },
        Expr::Paren(syn::ExprParen { expr: inner, .. })
        | Expr::Group(syn::ExprGroup { expr: inner, .. }) => literal_object_expr(inner),
        _ => None,
    }
}

fn float_literal_text(digits: &str) -> String {
    if digits.contains(['.', 'e', 'E']) {
        digits.to_owned()
    } else {
        format!("{digits}.0")
    }
}

/// Typed default for a literal, a negative literal, or the path `None`.
fn literal_default_repr(expr: &Expr) -> Option<TokenStream> {
    let repr = repr_path();
    match expr {
        Expr::Lit(syn::ExprLit { lit, .. }) => match lit {
            Lit::Bool(b) => {
                let v = b.value();
                Some(quote!(#repr::Bool(#v)))
            }
            Lit::Int(i) => {
                let v: i128 = i.base10_parse().ok()?;
                Some(quote!(#repr::Int(#v)))
            }
            Lit::Float(f) => {
                let text = float_literal_text(f.base10_digits());
                Some(quote!(#repr::Raw(#text)))
            }
            Lit::Str(s) => Some(quote!(#repr::Str(#s))),
            Lit::ByteStr(b) => Some(quote!(#repr::Bytes(#b))),
            Lit::Byte(b) => {
                let v = b.value();
                Some(quote!(#repr::Bytes(&[#v])))
            }
            Lit::Char(c) => {
                let s = c.value().to_string();
                Some(quote!(#repr::Str(#s)))
            }
            _ => None,
        },
        Expr::Path(path) if path.qself.is_none() && path.path.is_ident("None") => {
            Some(quote!(#repr::None))
        }
        Expr::Unary(syn::ExprUnary {
            op: syn::UnOp::Neg(_),
            expr: inner,
            ..
        }) => match inner.as_ref() {
            Expr::Lit(syn::ExprLit {
                lit: Lit::Int(i), ..
            }) => {
                let v: i128 = i.base10_parse().ok()?;
                let v = v.checked_neg()?;
                Some(quote!(#repr::Int(#v)))
            }
            Expr::Lit(syn::ExprLit {
                lit: Lit::Float(f), ..
            }) => {
                let text = float_literal_text(f.base10_digits());
                text.starts_with(|c: char| c.is_ascii_digit()).then(|| {
                    let text = format!("-{text}");
                    quote!(#repr::Raw(#text))
                })
            }
            _ => None,
        },
        Expr::Paren(syn::ExprParen { expr: inner, .. })
        | Expr::Group(syn::ExprGroup { expr: inner, .. }) => literal_default_repr(inner),
        _ => None,
    }
}

fn value_default_repr(field: &Field, expr: &Expr) -> TokenStream {
    if let Some(lit) = literal_default_repr(expr) {
        return lit;
    }
    let repr = repr_path();
    let ty = &field.ty;
    if is_primitive_int(ty) {
        quote!(#repr::Int((#expr) as i128))
    } else if is_bool_ty(ty) && matches!(expr, Expr::Path(_)) {
        quote!(#repr::Bool(#expr))
    } else {
        quote!({
            const V: #ty = #expr;
            V.py_default()
        })
    }
}

fn signature_default(field: &Field, attr: &ArgAttribute) -> Result<TokenStream> {
    let repr = repr_path();
    if let Some(text) = &attr.py_default {
        return Ok(quote!(Some(#repr::Raw(#text))));
    }
    if let Some(Some(expr)) = &attr.default {
        if let Some(name) = absolute_const_ident(expr)? {
            let name = name.to_string();
            return Ok(quote!(Some(#repr::Raw(#name))));
        }
        let value = value_default_repr(field, expr);
        return Ok(quote!(Some(#value)));
    }
    if attr.optional {
        let ty = &field.ty;
        return Ok(
            quote!(Some(<#ty as ::rustpython_vm::function::OptionalArgDefault>::PY_DEFAULT)),
        );
    }
    if attr.default.is_some() {
        // A bare `default` on a primitive is its zero value.
        let ty = &field.ty;
        let value = if is_primitive_int(ty) {
            quote!(#repr::Int(0))
        } else if is_bool_ty(ty) {
            quote!(#repr::Bool(false))
        } else if is_float_ty(ty) {
            quote!(#repr::Raw("0.0"))
        } else {
            quote!(#repr::Unrepresentable)
        };
        return Ok(quote!(Some(#value)));
    }
    Ok(quote!(None))
}

fn param_token(field: &Field, attr: &ArgAttribute) -> Result<TokenStream> {
    if let ParameterKind::Flatten = attr.kind {
        let ty = &field.ty;
        return Ok(quote! {
            ::rustpython_vm::function::Param::flatten(
                <#ty as ::rustpython_vm::function::FromArgs>::PARAMS
            )
        });
    }

    let name = field.ident.as_ref().map(|ident| ident.unraw().to_string());
    let pyname = attr
        .name
        .clone()
        .or(name)
        .ok_or_else(|| err_span!(field, "field in tuple struct must have name attribute"))?;
    let default_tok = signature_default(field, attr)?;
    let kind = match attr.kind {
        ParameterKind::PositionalOnly => {
            quote!(::rustpython_vm::function::ParamKind::PositionalOnly)
        }
        ParameterKind::PositionalOrKeyword => {
            quote!(::rustpython_vm::function::ParamKind::PositionalOrKeyword)
        }
        ParameterKind::KeywordOnly => quote!(::rustpython_vm::function::ParamKind::KeywordOnly),
        ParameterKind::Flatten => unreachable!(),
    };
    Ok(quote! {
        ::rustpython_vm::function::Param {
            name: #pyname,
            kind: #kind,
            default: #default_tok,
        }
    })
}

fn compute_arity_bounds(field_attrs: &[ArgAttribute]) -> (usize, usize) {
    let positional_fields = field_attrs.iter().filter(|attr| {
        matches!(
            attr.kind,
            ParameterKind::PositionalOnly | ParameterKind::PositionalOrKeyword
        )
    });

    let min_arity = positional_fields
        .clone()
        .filter(|attr| attr.default.is_none())
        .count();
    let max_arity = positional_fields.count();

    (min_arity, max_arity)
}

pub(crate) fn impl_from_args(input: DeriveInput) -> Result<TokenStream> {
    let Data::Struct(syn::DataStruct {
        fields: struct_fields,
        ..
    }) = input.data
    else {
        bail_span!(input, "FromArgs input must be a struct")
    };

    let fields = struct_fields
        .iter()
        .enumerate()
        .map(generate_field)
        .collect::<Result<TokenStream>>()?;
    let field_attrs = struct_fields
        .iter()
        .map(ArgAttribute::try_from)
        .collect::<Result<Vec<_>>>()?;
    let (min_arity, max_arity) = compute_arity_bounds(&field_attrs);

    let mut params = Vec::new();
    for field in &struct_fields {
        if is_phantom(field) {
            continue;
        }
        params.push(param_token(field, &ArgAttribute::try_from(field)?)?);
    }

    let name = input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    let output = quote! {
        impl #impl_generics ::rustpython_vm::function::FromArgs for #name #ty_generics #where_clause {
            const PARAMS: Option<&'static [::rustpython_vm::function::Param]> =
                Some(&[#(#params),*]);

            fn arity() -> ::std::ops::RangeInclusive<usize> {
                #min_arity..=#max_arity
            }

            fn from_args(
                vm: &::rustpython_vm::VirtualMachine,
                args: &mut ::rustpython_vm::function::FuncArgs
            ) -> ::core::result::Result<Self, ::rustpython_vm::function::ArgumentError> {
                Ok(Self { #fields })
            }
        }
    };
    Ok(output)
}
