use proc_macro2::TokenStream;
use quote::{ToTokens, format_ident, quote};
use syn::ext::IdentExt;
use syn::meta::ParseNestedMeta;
use syn::{
    Attribute, Data, DeriveInput, Expr, Field, Ident, Lit, Result, Token, Type, parse_quote,
};

#[cfg(test)]
mod integer_alias_tests;

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

#[derive(Clone, Copy)]
enum FieldMode {
    Clinic,
    Legacy,
}

fn try_from_object_expr(attr: &ArgAttribute, pyname: &str) -> TokenStream {
    if let Some(error_msg) = &attr.error_msg {
        return quote! {
            ::rustpython_vm::convert::TryFromObject::try_from_object(vm, x)
                .map_err(|_| vm.new_type_error(#error_msg))
        };
    }
    match attr.kind {
        ParameterKind::PositionalOnly => quote! {
            ::rustpython_vm::convert::TryFromObject::try_from_argument(vm, x)
        },
        _ => quote! {
            ::rustpython_vm::convert::TryFromObject::try_from_argument(vm, x)
                .map_err(|err| err.with_argument_name(#pyname))
        },
    }
}

fn default_ending_tokens(field: &Field, default: DefaultValue) -> Result<TokenStream> {
    let ty = &field.ty;
    let literal_obj = match &default {
        Some(expr) if !keeps_direct_literal(ty) && !is_wrapped_arg_ty(ty) => {
            literal_object_expr(expr)
        }
        _ => None,
    };
    if let Some(obj) = literal_obj {
        Ok(quote! {
            .map(<#ty as ::rustpython_vm::function::FromArgOptional>::from_inner)
            .map(::core::result::Result::Ok)
            .unwrap_or_else(|| {
                <#ty as ::rustpython_vm::convert::TryFromObject>::try_from_object(
                    vm,
                    ::rustpython_vm::convert::ToPyObject::to_pyobject(#obj, vm),
                )
                .map_err(::rustpython_vm::function::ArgumentError::from)
            })?
        })
    } else {
        let default = match default {
            Some(expr) => match absolute_const_ident(&expr)? {
                Some(name) => parse_quote!(::core::convert::Into::into(#name)),
                None => expr,
            },
            None => parse_quote!(::std::default::Default::default()),
        };
        Ok(quote! {
            .map(<#ty as ::rustpython_vm::function::FromArgOptional>::from_inner)
            .unwrap_or_else(|| #default)
        })
    }
}

fn missing_ending_tokens(kind: ParameterKind, pyname: &str, i: usize) -> TokenStream {
    // Flatten returns before this is called. A named-or-positional missing
    // argument is named back; a positional-only one is only counted.
    let err = if kind == ParameterKind::PositionalOnly {
        quote! {
            ::rustpython_vm::function::ArgumentError::TooFewArgs
        }
    } else {
        let pos = i + 1;
        quote! {
            ::rustpython_vm::function::ArgumentError::MissingRequiredArgument {
                name: #pyname.to_owned(),
                pos: #pos,
            }
        }
    };
    quote! {
        .ok_or_else(|| #err)?
    }
}

fn take_option_tokens(kind: ParameterKind, pyname: &str) -> TokenStream {
    if kind == ParameterKind::PositionalOnly {
        quote!(args.take_positional())
    } else if kind == ParameterKind::KeywordOnly {
        quote!(args.take_keyword(#pyname))
    } else {
        quote!(args.take_positional_keyword(#pyname))
    }
}

#[cfg(test)]
fn generate_field(item: (usize, &Field)) -> Result<TokenStream> {
    generate_field_with(item, FieldMode::Clinic)
}

fn generate_field_with((i, field): (usize, &Field), mode: FieldMode) -> Result<TokenStream> {
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
        .clone()
        .or(name_string)
        .ok_or_else(|| err_span!(field, "field in tuple struct must have name attribute"))?;
    let convert = try_from_object_expr(&attr, &pyname);

    if let FieldMode::Legacy = mode
        && attr.default.is_none()
    {
        return Ok(quote! {
            #field_name: {
                let x = slots.required(vm)?;
                #convert?
            },
        });
    }

    let middle = quote! { .map(|x| #convert).transpose()? };
    let taken = match mode {
        FieldMode::Clinic => take_option_tokens(attr.kind, &pyname),
        FieldMode::Legacy => quote!(slots.optional(vm)?),
    };
    let ending = if let Some(default) = attr.default {
        default_ending_tokens(field, default)?
    } else {
        missing_ending_tokens(attr.kind, &pyname, i)
    };

    Ok(quote! {
        #field_name: #taken #middle #ending,
    })
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
                | "PySsize"
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
    param_token_with(field, attr, &format_ident!("PARAMS"))
}

fn param_token_with(
    field: &Field,
    attr: &ArgAttribute,
    metadata: &syn::Ident,
) -> Result<TokenStream> {
    if let ParameterKind::Flatten = attr.kind {
        let ty = &field.ty;
        return Ok(quote! {
            ::rustpython_vm::function::Param::flatten(
                <#ty as ::rustpython_vm::function::FromArgs>::#metadata
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

struct LegacyFromArgs {
    fname: String,
}

fn parse_from_args_attr(attrs: &[Attribute]) -> Result<Option<LegacyFromArgs>> {
    let mut found = None;
    for attr in attrs {
        if !attr.path().is_ident("from_args") {
            continue;
        }
        if found.is_some() {
            bail_span!(attr, "multiple #[from_args] attributes");
        }
        found = Some(attr);
    }
    let Some(attr) = found else {
        return Ok(None);
    };

    let mut legacy = false;
    let mut fname = None;
    attr.parse_nested_meta(|meta| {
        if meta.path.is_ident("legacy") {
            if legacy {
                return Err(meta.error("duplicate `legacy`"));
            }
            legacy = true;
            Ok(())
        } else if meta.path.is_ident("fname") {
            if fname.is_some() {
                return Err(meta.error("duplicate `fname`"));
            }
            let val = meta.value()?.parse::<syn::LitStr>()?;
            fname = Some(val.value());
            Ok(())
        } else {
            Err(meta.error("unknown #[from_args] key; expected `legacy` or `fname = \"...\"`"))
        }
    })?;

    match (legacy, fname) {
        (true, Some(fname)) => Ok(Some(LegacyFromArgs { fname })),
        (true, None) => Err(err_span!(
            attr,
            "#[from_args(legacy)] requires fname = \"...\""
        )),
        (false, Some(_)) => Err(err_span!(
            attr,
            "#[from_args(fname = \"...\")] requires `legacy`"
        )),
        (false, None) => Err(err_span!(
            attr,
            "#[from_args] requires `legacy` and fname = \"...\""
        )),
    }
}

fn field_pyname(field: &Field, attr: &ArgAttribute) -> Result<String> {
    let name = field.ident.as_ref().map(|ident| ident.unraw().to_string());
    attr.name
        .clone()
        .or(name)
        .ok_or_else(|| err_span!(field, "field in tuple struct must have name attribute"))
}

fn validate_legacy_fields<'a>(fields: impl IntoIterator<Item = &'a Field>) -> Result<()> {
    let mut seen_optional = false;
    let mut seen_keyword_only = false;
    for field in fields {
        if is_phantom(field) {
            continue;
        }
        let attr = ArgAttribute::try_from(field)?;
        match attr.kind {
            ParameterKind::PositionalOnly => {
                bail_span!(
                    field,
                    "#[from_args(legacy)] does not support positional-only fields"
                );
            }
            ParameterKind::Flatten => {
                bail_span!(
                    field,
                    "#[from_args(legacy)] does not support flatten fields"
                );
            }
            ParameterKind::KeywordOnly => {
                seen_keyword_only = true;
            }
            ParameterKind::PositionalOrKeyword => {
                if seen_keyword_only {
                    bail_span!(
                        field,
                        "#[from_args(legacy)] does not allow a non-keyword-only field after a keyword-only field"
                    );
                }
            }
        }
        if attr.default.is_some() {
            seen_optional = true;
        } else if seen_optional {
            bail_span!(
                field,
                "#[from_args(legacy)] does not allow a required field after an optional one"
            );
        }
    }
    Ok(())
}

fn legacy_spec_parts<'a>(
    fields: impl IntoIterator<Item = &'a Field>,
) -> Result<(Vec<String>, usize, usize)> {
    let mut keywords = Vec::new();
    let mut required = 0;
    let mut max_positional = 0;
    for field in fields {
        if is_phantom(field) {
            continue;
        }
        let attr = ArgAttribute::try_from(field)?;
        keywords.push(field_pyname(field, &attr)?);
        if attr.default.is_none() {
            required += 1;
        }
        if attr.kind != ParameterKind::KeywordOnly {
            max_positional += 1;
        }
    }
    Ok((keywords, required, max_positional))
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

    let legacy = parse_from_args_attr(&input.attrs)?;
    if legacy.is_some() {
        validate_legacy_fields(&struct_fields)?;
    }

    let mode = if legacy.is_some() {
        FieldMode::Legacy
    } else {
        FieldMode::Clinic
    };
    let fields = struct_fields
        .iter()
        .enumerate()
        .map(|item| generate_field_with(item, mode))
        .collect::<Result<TokenStream>>()?;
    let field_attrs = struct_fields
        .iter()
        .map(ArgAttribute::try_from)
        .collect::<Result<Vec<_>>>()?;
    // Phantom fields are absent from both argument extraction and signatures.
    let visible_attrs = struct_fields
        .iter()
        .filter(|field| !is_phantom(field))
        .map(ArgAttribute::try_from)
        .collect::<Result<Vec<_>>>()?;
    let (min_arity, max_arity) = compute_arity_bounds(&visible_attrs);
    // Flattened types contribute both bounds. Saturate each sum, including
    // custom required bounds and an unbounded `PosArgs` upper bound.
    let flattened = struct_fields
        .iter()
        .zip(&field_attrs)
        .filter(|(field, attr)| !is_phantom(field) && attr.kind == ParameterKind::Flatten)
        .map(|(field, _)| &field.ty)
        .collect::<Vec<_>>();

    let mut params = Vec::new();
    for field in &struct_fields {
        if is_phantom(field) {
            continue;
        }
        params.push(param_token(field, &ArgAttribute::try_from(field)?)?);
    }

    let name = input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    let binding_params = if legacy.is_some() {
        quote! {
            const BINDING_PARAMS: Option<&'static [::rustpython_vm::function::Param]> = None;
        }
    } else if flattened.is_empty() {
        quote! {}
    } else {
        let params = struct_fields
            .iter()
            .filter(|field| !is_phantom(field))
            .map(|field| {
                param_token_with(
                    field,
                    &ArgAttribute::try_from(field)?,
                    &format_ident!("BINDING_PARAMS"),
                )
            })
            .collect::<Result<Vec<_>>>()?;
        quote! {
            const BINDING_PARAMS: Option<&'static [::rustpython_vm::function::Param]> =
                Some(&[#(#params),*]);
        }
    };
    let check_before_convert = if legacy.is_some() {
        quote! {
            fn check_before_convert(
                _: &::rustpython_vm::function::FuncArgs,
                _: &::std::ops::RangeInclusive<usize>,
                _: usize,
                _: usize,
                _: ::rustpython_vm::function::Callee,
                _: &::rustpython_vm::VirtualMachine,
            ) -> ::core::option::Option<::rustpython_vm::builtins::PyBaseExceptionRef> {
                None
            }
        }
    } else {
        quote! {}
    };
    let from_args_body = if let Some(LegacyFromArgs { fname }) = legacy {
        let (keywords, required, max_positional) = legacy_spec_parts(&struct_fields)?;
        quote! {
            const SPEC: ::rustpython_vm::function::ArgSpec<'static> =
                ::rustpython_vm::function::ArgSpec {
                    fname: #fname,
                    keywords: &[#(#keywords),*],
                    required: #required,
                    max_positional: #max_positional,
                };
            let bound = SPEC.bind(args, vm, |slots| -> ::core::result::Result<
                Self,
                ::rustpython_vm::function::ArgumentError,
            > { Ok(Self { #fields }) })?;
            *args = ::core::default::Default::default();
            Ok(bound)
        }
    } else {
        quote! {
            Ok(Self { #fields })
        }
    };
    Ok(quote! {
        impl #impl_generics ::rustpython_vm::function::FromArgs for #name #ty_generics #where_clause {
            const PARAMS: Option<&'static [::rustpython_vm::function::Param]> =
                Some(&[#(#params),*]);

            #binding_params

            const HAS_REQUIRED_KEYWORD_ONLY: bool =
                ::rustpython_vm::function::params_have_required_keyword_only(Self::PARAMS) #(
                    || <#flattened as ::rustpython_vm::function::FromArgs>::HAS_REQUIRED_KEYWORD_ONLY
                )*;

            fn arity() -> ::std::ops::RangeInclusive<usize> {
                let min = #min_arity;
                let max = #max_arity;
                #(
                    let (start, end) = <#flattened as ::rustpython_vm::function::FromArgs>::arity().into_inner();
                    let min = min.saturating_add(start);
                    let max = max.saturating_add(end);
                )*
                min..=max
            }

            #check_before_convert

            fn from_args(
                vm: &::rustpython_vm::VirtualMachine,
                args: &mut ::rustpython_vm::function::FuncArgs
            ) -> ::core::result::Result<Self, ::rustpython_vm::function::ArgumentError> {
                #from_args_body
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn option_none_default_keeps_rust_value_and_metadata() {
        let types: [Type; 2] = [
            parse_quote!(Option<PyObjectRef>),
            parse_quote!(Option<PyStrRef>),
        ];
        for ty in types {
            let field: Field = parse_quote! {
                #[pyarg(any, default = None)]
                value: #ty
            };
            let tokens = generate_field((0, &field)).unwrap().to_string();
            assert!(tokens.contains(&quote!(unwrap_or_else(|| None)).to_string()));
            assert!(!tokens.contains("vm . ctx . none"));
            let attr = ArgAttribute::try_from(&field).unwrap();
            let repr = repr_path();
            assert_eq!(
                signature_default(&field, &attr).unwrap().to_string(),
                quote!(Some(#repr::None)).to_string()
            );
        }
    }

    #[test]
    fn none_default_keeps_wrapped_argument_paths() {
        let types: [Type; 3] = [
            parse_quote!(Option<PyObjectRef>),
            parse_quote!(OptionalArg<PyObjectRef>),
            parse_quote!(OptionalOption<PyObjectRef>),
        ];
        for ty in types {
            let field: Field = parse_quote! {
                #[pyarg(any, default = None)]
                value: #ty
            };
            let tokens = generate_field((0, &field)).unwrap().to_string();
            assert!(!tokens.contains("vm . ctx . none"));
            assert!(tokens.contains(&quote!(unwrap_or_else(|| None)).to_string()));
        }
    }

    #[test]
    fn none_is_not_a_python_object_literal() {
        for expr in [parse_quote!(None), parse_quote!((None))] {
            assert!(literal_object_expr(&expr).is_none());
        }
        for expr in [
            parse_quote!("text"),
            parse_quote!(b"bytes"),
            parse_quote!(true),
        ] {
            assert_eq!(
                literal_object_expr(&expr).unwrap().to_string(),
                quote!(#expr).to_string()
            );
        }
        let field: Field = parse_quote! {
            #[pyarg(any, default = "text")]
            value: PyStrRef
        };
        let tokens = generate_field((0, &field)).unwrap().to_string();
        assert!(tokens.contains("< PyStrRef as :: rustpython_vm :: convert :: TryFromObject >"));
        assert!(!tokens.contains("vm . ctx . none"));
    }

    fn tokens_of(input: DeriveInput) -> String {
        match impl_from_args(input) {
            Ok(tokens) => tokens.to_string(),
            Err(err) => panic!("{err}"),
        }
    }

    fn error_of(input: DeriveInput) -> String {
        match impl_from_args(input) {
            Ok(_) => panic!("expected a compile error"),
            Err(err) => err.to_string(),
        }
    }

    #[test]
    fn without_from_args_attribute_does_not_use_arg_spec() {
        let input: DeriveInput = parse_quote! {
            struct ExampleArgs {
                #[pyarg(any)]
                first: i32,
            }
        };
        let tokens = tokens_of(input);
        assert!(!tokens.contains("ArgSpec"));
        assert!(!tokens.contains("check_before_convert"));
        assert!(tokens.contains("take_positional_keyword"));
    }

    #[test]
    fn legacy_from_args_binds_with_arg_spec() {
        let input: DeriveInput = parse_quote! {
            #[from_args(legacy, fname = "example")]
            struct ExampleArgs {
                #[pyarg(any)]
                first: i32,
                #[pyarg(any, default)]
                second: i32,
                #[pyarg(named, default)]
                third: i32,
            }
        };
        let tokens = tokens_of(input);
        assert!(tokens.contains("ArgSpec"));
        assert!(tokens.contains("example"));
        assert!(tokens.contains("check_before_convert"));
        assert!(tokens.contains("slots . required"));
        assert!(tokens.contains("optional"));
        assert!(tokens.contains("first"));
        assert!(tokens.contains("second"));
        assert!(tokens.contains("third"));
    }

    #[test]
    fn legacy_without_fname_is_an_error() {
        let input: DeriveInput = parse_quote! {
            #[from_args(legacy)]
            struct ExampleArgs {
                #[pyarg(any)]
                first: i32,
            }
        };
        let err = error_of(input);
        assert!(err.contains("fname"), "{err}");
    }

    #[test]
    fn fname_without_legacy_is_an_error() {
        let input: DeriveInput = parse_quote! {
            #[from_args(fname = "example")]
            struct ExampleArgs {
                #[pyarg(any)]
                first: i32,
            }
        };
        let err = error_of(input);
        assert!(err.contains("legacy"), "{err}");
    }

    #[test]
    fn unknown_from_args_key_is_an_error() {
        let input: DeriveInput = parse_quote! {
            #[from_args(legacy, fname = "example", extra)]
            struct ExampleArgs {
                #[pyarg(any)]
                first: i32,
            }
        };
        let err = error_of(input);
        assert!(err.contains("unknown"), "{err}");
    }

    #[test]
    fn legacy_rejects_positional_fields() {
        let input: DeriveInput = parse_quote! {
            #[from_args(legacy, fname = "example")]
            struct ExampleArgs {
                #[pyarg(positional)]
                first: i32,
            }
        };
        let err = error_of(input);
        assert!(err.contains("positional-only"), "{err}");
    }

    #[test]
    fn legacy_rejects_flatten_fields() {
        let input: DeriveInput = parse_quote! {
            #[from_args(legacy, fname = "example")]
            struct ExampleArgs {
                #[pyarg(flatten)]
                first: i32,
            }
        };
        let err = error_of(input);
        assert!(err.contains("flatten"), "{err}");
    }

    #[test]
    fn legacy_rejects_required_after_optional() {
        let input: DeriveInput = parse_quote! {
            #[from_args(legacy, fname = "example")]
            struct ExampleArgs {
                #[pyarg(any, default)]
                first: i32,
                #[pyarg(any)]
                second: i32,
            }
        };
        let err = error_of(input);
        assert!(err.contains("required field after an optional"), "{err}");
    }

    #[test]
    fn legacy_rejects_non_keyword_only_after_named() {
        let input: DeriveInput = parse_quote! {
            #[from_args(legacy, fname = "example")]
            struct ExampleArgs {
                #[pyarg(named, default)]
                first: i32,
                #[pyarg(any, default)]
                second: i32,
            }
        };
        let err = error_of(input);
        assert!(
            err.contains("non-keyword-only field after a keyword-only"),
            "{err}"
        );
    }
}
