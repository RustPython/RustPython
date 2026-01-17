use super::Diagnostic;
use crate::util::{
    ALL_ALLOWED_NAMES, ClassItemMeta, ContentItem, ContentItemInner, ErrorVec, ExceptionItemMeta,
    ItemMeta, ItemMetaInner, ItemNursery, SimpleItemMeta, format_doc, pyclass_ident_and_attrs,
    pyexception_ident_and_attrs, text_signature,
};
use core::str::FromStr;
use proc_macro2::{Delimiter, Group, Span, TokenStream, TokenTree};
use quote::{ToTokens, quote, quote_spanned};
use rustpython_doc::DB;
use std::collections::{HashMap, HashSet};
use syn::{Attribute, Ident, Item, Result, parse_quote, spanned::Spanned};
use syn_ext::ext::*;
use syn_ext::types::*;

#[derive(Copy, Clone, Debug)]
enum AttrName {
    Method,
    ClassMethod,
    StaticMethod,
    GetSet,
    Slot,
    Attr,
    ExtendClass,
    Member,
}

impl core::fmt::Display for AttrName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let s = match self {
            Self::Method => "pymethod",
            Self::ClassMethod => "pyclassmethod",
            Self::StaticMethod => "pystaticmethod",
            Self::GetSet => "pygetset",
            Self::Slot => "pyslot",
            Self::Attr => "pyattr",
            Self::ExtendClass => "extend_class",
            Self::Member => "pymember",
        };
        s.fmt(f)
    }
}

impl FromStr for AttrName {
    type Err = String;

    fn from_str(s: &str) -> core::result::Result<Self, Self::Err> {
        Ok(match s {
            "pymethod" => Self::Method,
            "pyclassmethod" => Self::ClassMethod,
            "pystaticmethod" => Self::StaticMethod,
            "pygetset" => Self::GetSet,
            "pyslot" => Self::Slot,
            "pyattr" => Self::Attr,
            "extend_class" => Self::ExtendClass,
            "pymember" => Self::Member,
            s => {
                return Err(s.to_owned());
            }
        })
    }
}

#[derive(Default)]
struct ImplContext {
    is_trait: bool,
    attribute_items: ItemNursery,
    method_items: MethodNursery,
    getset_items: GetSetNursery,
    member_items: MemberNursery,
    extend_slots_items: ItemNursery,
    class_extensions: Vec<TokenStream>,
    errors: Vec<syn::Error>,
}

fn extract_items_into_context<'a, Item>(
    context: &mut ImplContext,
    items: impl Iterator<Item = &'a mut Item>,
) where
    Item: ItemLike + ToTokens + GetIdent + syn_ext::ext::ItemAttrExt + 'a,
{
    for item in items {
        let r = item.try_split_attr_mut(|attrs, item| {
            let (py_items, cfgs) = attrs_to_content_items(attrs, impl_item_new::<Item>)?;
            for py_item in py_items.iter().rev() {
                let r = py_item.gen_impl_item(ImplItemArgs::<Item> {
                    item,
                    attrs,
                    context,
                    cfgs: cfgs.as_slice(),
                });
                context.errors.ok_or_push(r);
            }
            Ok(())
        });
        context.errors.ok_or_push(r);
    }
    context.errors.ok_or_push(context.method_items.validate());
    context.errors.ok_or_push(context.getset_items.validate());
    context.errors.ok_or_push(context.member_items.validate());
}

pub(crate) fn impl_pyclass_impl(attr: PunctuatedNestedMeta, item: Item) -> Result<TokenStream> {
    let mut context = ImplContext::default();
    let mut tokens = match item {
        Item::Impl(mut imp) => {
            extract_items_into_context(&mut context, imp.items.iter_mut());

            let (impl_ty, payload_guess) = match imp.self_ty.as_ref() {
                syn::Type::Path(syn::TypePath {
                    path: syn::Path { segments, .. },
                    ..
                }) if segments.len() == 1 => {
                    let segment = &segments[0];
                    let payload_ty = if segment.ident == "Py" || segment.ident == "PyRef" {
                        match &segment.arguments {
                            syn::PathArguments::AngleBracketed(
                                syn::AngleBracketedGenericArguments { args, .. },
                            ) if args.len() == 1 => {
                                let arg = &args[0];
                                match arg {
                                    syn::GenericArgument::Type(syn::Type::Path(
                                        syn::TypePath {
                                            path: syn::Path { segments, .. },
                                            ..
                                        },
                                    )) if segments.len() == 1 => segments[0].ident.clone(),
                                    _ => {
                                        return Err(syn::Error::new_spanned(
                                            segment,
                                            "Py{Ref}<T> is expected but Py{Ref}<?> is found",
                                        ));
                                    }
                                }
                            }
                            _ => {
                                return Err(syn::Error::new_spanned(
                                    segment,
                                    "Py{Ref}<T> is expected but Py{Ref}? is found",
                                ));
                            }
                        }
                    } else {
                        if !matches!(segment.arguments, syn::PathArguments::None) {
                            return Err(syn::Error::new_spanned(
                                segment,
                                "PyImpl can only be implemented for Py{Ref}<T> or T",
                            ));
                        }
                        segment.ident.clone()
                    };
                    (segment.ident.clone(), payload_ty)
                }
                _ => {
                    return Err(syn::Error::new_spanned(
                        imp.self_ty,
                        "PyImpl can only be implemented for Py{Ref}<T> or T",
                    ));
                }
            };

            let ExtractedImplAttrs {
                payload: attr_payload,
                flags,
                with_impl,
                with_method_defs,
                with_slots,
                itemsize,
            } = extract_impl_attrs(attr, &impl_ty)?;
            let payload_ty = attr_payload.unwrap_or(payload_guess);
            let method_def = &context.method_items;
            let getset_impl = &context.getset_items;
            let member_impl = &context.member_items;
            let extend_impl = context.attribute_items.validate()?;
            let slots_impl = context.extend_slots_items.validate()?;
            let class_extensions = &context.class_extensions;

            let extra_methods = [
                parse_quote! {
                    const __OWN_METHOD_DEFS: &'static [::rustpython_vm::function::PyMethodDef] = &#method_def;
                },
                parse_quote! {
                    fn __extend_py_class(
                        ctx: &::rustpython_vm::Context,
                        class: &'static ::rustpython_vm::Py<::rustpython_vm::builtins::PyType>,
                    ) {
                        #getset_impl
                        #member_impl
                        #extend_impl
                        #(#class_extensions)*
                    }
                },
                {
                    let itemsize_impl = itemsize.as_ref().map(|size| {
                        quote! {
                            slots.itemsize = #size;
                        }
                    });
                    parse_quote! {
                        fn __extend_slots(slots: &mut ::rustpython_vm::types::PyTypeSlots) {
                            #itemsize_impl
                            #slots_impl
                        }
                    }
                },
            ];
            imp.items.extend(extra_methods);
            let is_main_impl = impl_ty == payload_ty;
            if is_main_impl {
                let method_defs = if with_method_defs.is_empty() {
                    quote!(#impl_ty::__OWN_METHOD_DEFS)
                } else {
                    quote!(
                        rustpython_vm::function::PyMethodDef::__const_concat_arrays::<
                            { #impl_ty::__OWN_METHOD_DEFS.len() #(+ #with_method_defs.len())* },
                        >(&[#impl_ty::__OWN_METHOD_DEFS, #(#with_method_defs,)*])
                    )
                };
                quote! {
                    #imp
                    impl ::rustpython_vm::class::PyClassImpl for #payload_ty {
                        const TP_FLAGS: ::rustpython_vm::types::PyTypeFlags = #flags;

                        fn impl_extend_class(
                            ctx: &::rustpython_vm::Context,
                            class: &'static ::rustpython_vm::Py<::rustpython_vm::builtins::PyType>,
                        ) {
                            #impl_ty::__extend_py_class(ctx, class);
                            #with_impl
                        }

                        const METHOD_DEFS: &'static [::rustpython_vm::function::PyMethodDef] = &#method_defs;

                        fn extend_slots(slots: &mut ::rustpython_vm::types::PyTypeSlots) {
                            #with_slots
                            #impl_ty::__extend_slots(slots);
                        }
                    }
                }
            } else {
                imp.into_token_stream()
            }
        }
        Item::Trait(mut trai) => {
            let mut context = ImplContext {
                is_trait: true,
                ..Default::default()
            };
            let mut has_extend_slots = false;
            for item in &trai.items {
                let has = match item {
                    syn::TraitItem::Fn(item) => item.sig.ident == "extend_slots",
                    _ => false,
                };
                if has {
                    has_extend_slots = has;
                    break;
                }
            }
            extract_items_into_context(&mut context, trai.items.iter_mut());

            let ExtractedImplAttrs {
                with_impl,
                with_slots,
                ..
            } = extract_impl_attrs(attr, &trai.ident)?;

            let method_def = &context.method_items;
            let getset_impl = &context.getset_items;
            let member_impl = &context.member_items;
            let extend_impl = &context.attribute_items.validate()?;
            let slots_impl = &context.extend_slots_items.validate()?;
            let class_extensions = &context.class_extensions;
            let call_extend_slots = if has_extend_slots {
                quote! {
                    Self::extend_slots(slots);
                }
            } else {
                quote! {}
            };
            let extra_methods = [
                parse_quote! {
                    const __OWN_METHOD_DEFS: &'static [::rustpython_vm::function::PyMethodDef] = &#method_def;
                },
                parse_quote! {
                    fn __extend_py_class(
                        ctx: &::rustpython_vm::Context,
                        class: &'static ::rustpython_vm::Py<::rustpython_vm::builtins::PyType>,
                    ) {
                        #getset_impl
                        #member_impl
                        #extend_impl
                        #with_impl
                        #(#class_extensions)*
                    }
                },
                parse_quote! {
                    fn __extend_slots(slots: &mut ::rustpython_vm::types::PyTypeSlots) {
                        #with_slots
                        #slots_impl
                        #call_extend_slots
                    }
                },
            ];
            trai.items.extend(extra_methods);

            trai.into_token_stream()
        }
        item => item.into_token_stream(),
    };
    if let Some(error) = context.errors.into_error() {
        let error = Diagnostic::from(error);
        tokens = quote! {
            #tokens
            #error
        }
    }
    Ok(tokens)
}

/// Validates that when a base class is specified, the struct has the base type as its first field.
/// This ensures proper memory layout for subclassing (required for #[repr(transparent)] to work correctly).
fn validate_base_field(item: &Item, base_path: &syn::Path) -> Result<()> {
    let Item::Struct(item_struct) = item else {
        // Only validate structs - enums with base are already an error elsewhere
        return Ok(());
    };

    // Get the base type name for error messages
    let base_name = base_path
        .segments
        .last()
        .map(|s| s.ident.to_string())
        .unwrap_or_else(|| quote!(#base_path).to_string());

    match &item_struct.fields {
        syn::Fields::Named(fields) => {
            let Some(first_field) = fields.named.first() else {
                bail_span!(
                    item_struct,
                    "#[pyclass] with base = {base_name} requires the first field to be of type {base_name}, but the struct has no fields"
                );
            };
            if !type_matches_path(&first_field.ty, base_path) {
                bail_span!(
                    first_field,
                    "#[pyclass] with base = {base_name} requires the first field to be of type {base_name}"
                );
            }
        }
        syn::Fields::Unnamed(fields) => {
            let Some(first_field) = fields.unnamed.first() else {
                bail_span!(
                    item_struct,
                    "#[pyclass] with base = {base_name} requires the first field to be of type {base_name}, but the struct has no fields"
                );
            };
            if !type_matches_path(&first_field.ty, base_path) {
                bail_span!(
                    first_field,
                    "#[pyclass] with base = {base_name} requires the first field to be of type {base_name}"
                );
            }
        }
        syn::Fields::Unit => {
            bail_span!(
                item_struct,
                "#[pyclass] with base = {base_name} requires the first field to be of type {base_name}, but the struct is a unit struct"
            );
        }
    }

    Ok(())
}

/// Check if a type matches a given path (handles simple cases like `Foo` or `path::to::Foo`)
fn type_matches_path(ty: &syn::Type, path: &syn::Path) -> bool {
    // Compare by converting both to string representation for macro hygiene
    let ty_str = quote!(#ty).to_string().replace(' ', "");
    let path_str = quote!(#path).to_string().replace(' ', "");

    // Check if both are the same or if the type ends with the path's last segment
    if ty_str == path_str {
        return true;
    }

    // Also match if just the last segment matches (e.g., foo::Bar matches Bar)
    let syn::Type::Path(type_path) = ty else {
        return false;
    };
    let Some(type_last) = type_path.path.segments.last() else {
        return false;
    };
    let Some(path_last) = path.segments.last() else {
        return false;
    };
    type_last.ident == path_last.ident
}

fn generate_class_def(
    ident: &Ident,
    name: &str,
    module_name: Option<&str>,
    base: Option<syn::Path>,
    metaclass: Option<String>,
    unhashable: bool,
    attrs: &[Attribute],
) -> Result<TokenStream> {
    let doc = attrs.doc().or_else(|| {
        let module_name = module_name.unwrap_or("builtins");
        DB.get(&format!("{module_name}.{name}"))
            .copied()
            .map(str::to_owned)
    });
    let doc = if let Some(doc) = doc {
        quote!(Some(#doc))
    } else {
        quote!(None)
    };
    let module_class_name = if let Some(module_name) = module_name {
        format!("{module_name}.{name}")
    } else {
        name.to_owned()
    };
    let module_name = match module_name {
        Some(v) => quote!(Some(#v) ),
        None => quote!(None),
    };
    let unhashable = if unhashable {
        quote!(true)
    } else {
        quote!(false)
    };
    let is_pystruct = attrs.iter().any(|attr| {
        attr.path().is_ident("derive")
            && if let Ok(Meta::List(l)) = attr.parse_meta() {
                l.nested
                    .into_iter()
                    .any(|n| n.get_ident().is_some_and(|p| p == "PyStructSequence"))
            } else {
                false
            }
    });
    // Check if the type has #[repr(transparent)] - only then we can safely
    // generate PySubclass impl (requires same memory layout as base type)
    let is_repr_transparent = attrs.iter().any(|attr| {
        attr.path().is_ident("repr")
            && if let Ok(Meta::List(l)) = attr.parse_meta() {
                l.nested
                    .into_iter()
                    .any(|n| n.get_ident().is_some_and(|p| p == "transparent"))
            } else {
                false
            }
    });
    // If repr(transparent) with a base, the type has the same memory layout as base,
    // so basicsize should be 0 (no additional space beyond the base type)
    // Otherwise, basicsize = sizeof(payload). The header size is added in __basicsize__ getter.
    let basicsize = if is_repr_transparent && base.is_some() {
        quote!(0)
    } else {
        quote!(std::mem::size_of::<#ident>())
    };
    if base.is_some() && is_pystruct {
        bail_span!(ident, "PyStructSequence cannot have `base` class attr",);
    }
    let base_class = if is_pystruct {
        Some(quote! { rustpython_vm::builtins::PyTuple })
    } else {
        base.as_ref().map(|typ| {
            quote_spanned! { ident.span() => #typ }
        })
    }
    .map(|typ| {
        quote! {
            fn static_baseclass() -> &'static ::rustpython_vm::Py<::rustpython_vm::builtins::PyType> {
                use rustpython_vm::class::StaticType;
                #typ::static_type()
            }
        }
    });

    let meta_class = metaclass.map(|typ| {
        let typ = Ident::new(&typ, ident.span());
        quote! {
            fn static_metaclass() -> &'static ::rustpython_vm::Py<::rustpython_vm::builtins::PyType> {
                use rustpython_vm::class::StaticType;
                #typ::static_type()
            }
        }
    });

    let base_or_object = if let Some(ref base) = base {
        quote! { #base }
    } else {
        quote! { ::rustpython_vm::builtins::PyBaseObject }
    };

    // Generate PySubclass impl for #[repr(transparent)] types with base class
    // (tuple struct assumed, so &self.0 works)
    let subclass_impl = if !is_pystruct && is_repr_transparent {
        base.as_ref().map(|typ| {
            quote! {
                impl ::rustpython_vm::class::PySubclass for #ident {
                    type Base = #typ;

                    #[inline]
                    fn as_base(&self) -> &Self::Base {
                        &self.0
                    }
                }
            }
        })
    } else {
        None
    };

    let tokens = quote! {
        impl ::rustpython_vm::class::PyClassDef for #ident {
            const NAME: &'static str = #name;
            const MODULE_NAME: Option<&'static str> = #module_name;
            const TP_NAME: &'static str = #module_class_name;
            const DOC: Option<&'static str> = #doc;
            const BASICSIZE: usize = #basicsize;
            const UNHASHABLE: bool = #unhashable;

            type Base = #base_or_object;
        }

        impl ::rustpython_vm::class::StaticType for #ident {
            fn static_cell() -> &'static ::rustpython_vm::common::static_cell::StaticCell<::rustpython_vm::builtins::PyTypeRef> {
                ::rustpython_vm::common::static_cell! {
                    static CELL: ::rustpython_vm::builtins::PyTypeRef;
                }
                &CELL
            }

            #meta_class

            #base_class
        }

        #subclass_impl
    };
    Ok(tokens)
}

pub(crate) fn impl_pyclass(attr: PunctuatedNestedMeta, item: Item) -> Result<TokenStream> {
    if matches!(item, syn::Item::Use(_)) {
        return Ok(quote!(#item));
    }
    let (ident, attrs) = pyclass_ident_and_attrs(&item)?;
    let fake_ident = Ident::new("pyclass", item.span());
    let class_meta = ClassItemMeta::from_nested(ident.clone(), fake_ident, attr.into_iter())?;
    let class_name = class_meta.class_name()?;
    let module_name = class_meta.module()?;
    let base = class_meta.base()?;
    let metaclass = class_meta.metaclass()?;
    let unhashable = class_meta.unhashable()?;

    // Validate that if base is specified, the first field must be of the base type
    if let Some(ref base_path) = base {
        validate_base_field(&item, base_path)?;
    }

    let class_def = generate_class_def(
        ident,
        &class_name,
        module_name.as_deref(),
        base.clone(),
        metaclass,
        unhashable,
        attrs,
    )?;

    const ALLOWED_TRAVERSE_OPTS: &[&str] = &["manual"];
    // try to know if it have a `#[pyclass(trace)]` exist on this struct
    // TODO(discord9): rethink on auto detect `#[Derive(PyTrace)]`

    // 1. no `traverse` at all: generate a dummy try_traverse
    // 2. `traverse = "manual"`: generate a try_traverse, but not #[derive(Traverse)]
    // 3. `traverse`: generate a try_traverse, and #[derive(Traverse)]
    let (maybe_trace_code, derive_trace) = {
        if class_meta.inner()._has_key("traverse")? {
            let maybe_trace_code = quote! {
                impl ::rustpython_vm::object::MaybeTraverse for #ident {
                    const IS_TRACE: bool = true;
                    fn try_traverse(&self, tracer_fn: &mut ::rustpython_vm::object::TraverseFn) {
                        ::rustpython_vm::object::Traverse::traverse(self, tracer_fn);
                    }
                }
            };
            // if the key `traverse` exist but not as key-value, _optional_str return Err(...)
            // so we need to check if it is Ok(Some(...))
            let value = class_meta.inner()._optional_str("traverse");
            let derive_trace = if let Ok(Some(s)) = value {
                if !ALLOWED_TRAVERSE_OPTS.contains(&s.as_str()) {
                    bail_span!(
                        item,
                        "traverse attribute only accept {ALLOWED_TRAVERSE_OPTS:?} as value or no value at all",
                    );
                }
                assert_eq!(s, "manual");
                quote! {}
            } else {
                quote! {#[derive(Traverse)]}
            };
            (maybe_trace_code, derive_trace)
        } else {
            (
                // a dummy impl, which do nothing
                // #attrs
                quote! {
                    impl ::rustpython_vm::object::MaybeTraverse for #ident {
                        fn try_traverse(&self, tracer_fn: &mut ::rustpython_vm::object::TraverseFn) {
                            // do nothing
                        }
                    }
                },
                quote! {},
            )
        }
    };

    // Generate PyPayload impl based on whether base exists
    #[allow(clippy::collapsible_else_if)]
    let impl_payload = if let Some(base_type) = &base {
        let class_fn = if let Some(ctx_type_name) = class_meta.ctx_name()? {
            let ctx_type_ident = Ident::new(&ctx_type_name, ident.span());
            quote! { ctx.types.#ctx_type_ident }
        } else {
            quote! { <Self as ::rustpython_vm::class::StaticType>::static_type() }
        };

        quote! {
            // static_assertions::const_assert!(std::mem::size_of::<#base_type>() <= std::mem::size_of::<#ident>());
            impl ::rustpython_vm::PyPayload for #ident {
                const PAYLOAD_TYPE_ID: ::core::any::TypeId = <#base_type as ::rustpython_vm::PyPayload>::PAYLOAD_TYPE_ID;

                #[inline]
                fn validate_downcastable_from(obj: &::rustpython_vm::PyObject) -> bool {
                    <Self as ::rustpython_vm::class::PyClassDef>::BASICSIZE <= obj.class().slots.basicsize && obj.class().fast_issubclass(<Self as ::rustpython_vm::class::StaticType>::static_type())
                }

                fn class(ctx: &::rustpython_vm::vm::Context) -> &'static ::rustpython_vm::Py<::rustpython_vm::builtins::PyType> {
                    #class_fn
                }
            }
        }
    } else {
        if let Some(ctx_type_name) = class_meta.ctx_name()? {
            let ctx_type_ident = Ident::new(&ctx_type_name, ident.span());
            quote! {
                impl ::rustpython_vm::PyPayload for #ident {
                    fn class(ctx: &::rustpython_vm::vm::Context) -> &'static ::rustpython_vm::Py<::rustpython_vm::builtins::PyType> {
                        ctx.types.#ctx_type_ident
                    }
                }
            }
        } else {
            quote! {}
        }
    };

    let empty_impl = if let Some(attrs) = class_meta.impl_attrs()? {
        let attrs: Meta = parse_quote! (#attrs);
        quote! {
            #[pyclass(#attrs)]
            impl #ident {}
        }
    } else {
        quote! {}
    };

    let ret = quote! {
        #derive_trace
        #item
        #maybe_trace_code
        #class_def
        #impl_payload
        #empty_impl
    };
    Ok(ret)
}

/// Special macro to create exception types.
///
/// Why do we need it and why can't we just use `pyclass` macro instead?
/// We generate exception types with a `macro_rules`,
/// similar to how CPython does it.
/// But, inside `macro_rules` we don't have an opportunity
/// to add non-literal attributes to `pyclass`.
/// That's why we have to use this proxy.
pub(crate) fn impl_pyexception(attr: PunctuatedNestedMeta, item: Item) -> Result<TokenStream> {
    let (ident, _attrs) = pyexception_ident_and_attrs(&item)?;
    let fake_ident = Ident::new("pyclass", item.span());
    let class_meta = ExceptionItemMeta::from_nested(ident.clone(), fake_ident, attr.into_iter())?;
    let class_name = class_meta.class_name()?;

    let base_class_name = class_meta.base()?;
    let impl_pyclass = if class_meta.has_impl()? {
        quote! {
            #[pyexception]
            impl #ident {}
        }
    } else {
        quote! {}
    };

    let ret = quote! {
        #[pyclass(module = false, name = #class_name, base = #base_class_name)]
        #item
        #impl_pyclass
    };
    Ok(ret)
}

pub(crate) fn impl_pyexception_impl(attr: PunctuatedNestedMeta, item: Item) -> Result<TokenStream> {
    let Item::Impl(imp) = item else {
        return Ok(item.into_token_stream());
    };

    // Check if with(Constructor) is specified. If Constructor trait is used, don't generate slot_new
    let mut extra_attrs = Vec::new();
    let mut with_items = vec![];
    for nested in &attr {
        if let NestedMeta::Meta(Meta::List(MetaList { path, nested, .. })) = nested {
            // If we already found the constructor trait, no need to keep looking for it
            if path.is_ident("with") {
                for meta in nested {
                    with_items.push(meta.get_ident().expect("with() has non-ident item").clone());
                }
                continue;
            }
            extra_attrs.push(NestedMeta::Meta(Meta::List(MetaList {
                path: path.clone(),
                paren_token: Default::default(),
                nested: nested.clone(),
            })));
        }
    }

    let with_contains = |with_items: &[Ident], s: &str| {
        // Check if Constructor is in the list
        with_items.iter().any(|ident| ident == s)
    };

    let syn::ItemImpl {
        generics,
        self_ty,
        items,
        ..
    } = &imp;

    let slot_new = if with_contains(&with_items, "Constructor") {
        quote!()
    } else {
        with_items.push(Ident::new("Constructor", Span::call_site()));
        quote! {
            impl ::rustpython_vm::types::Constructor for #self_ty {
                type Args = ::rustpython_vm::function::FuncArgs;

                fn slot_new(
                    cls: ::rustpython_vm::builtins::PyTypeRef,
                    args: ::rustpython_vm::function::FuncArgs,
                    vm: &::rustpython_vm::VirtualMachine,
                ) -> ::rustpython_vm::PyResult {
                    <Self as ::rustpython_vm::class::PyClassDef>::Base::slot_new(cls, args, vm)
                }
                fn py_new(
                    _cls: &::rustpython_vm::Py<::rustpython_vm::builtins::PyType>,
                    _args: Self::Args,
                    _vm: &::rustpython_vm::VirtualMachine
                ) -> ::rustpython_vm::PyResult<Self> {
                    unreachable!("slot_new is defined")
                }
            }
        }
    };

    // SimpleExtendsException: inherits BaseException_init from the base class via MRO.
    // Only exceptions that explicitly specify `with(Initializer)` will have
    // their own __init__ in __dict__.
    let slot_init = quote!();

    let extra_attrs_tokens = if extra_attrs.is_empty() {
        quote!()
    } else {
        quote!(, #(#extra_attrs),*)
    };

    Ok(quote! {
        #[pyclass(flags(BASETYPE, HAS_DICT), with(#(#with_items),*) #extra_attrs_tokens)]
        impl #generics #self_ty {
            #(#items)*
        }

        #slot_new
        #slot_init
    })
}

macro_rules! define_content_item {
    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident
    ) => {
        $(#[$meta])*
        $vis struct $name {
            inner: ContentItemInner<AttrName>,
        }

        impl ContentItem for $name {
            type AttrName = AttrName;

            fn inner(&self) -> &ContentItemInner<AttrName> {
                &self.inner
            }
        }
    };
}

define_content_item!(
    /// #[pymethod] and #[pyclassmethod]
    struct MethodItem
);

define_content_item!(
    /// #[pygetset]
    struct GetSetItem
);

define_content_item!(
    /// #[pyslot]
    struct SlotItem
);

define_content_item!(
    /// #[pyattr]
    struct AttributeItem
);

define_content_item!(
    /// #[extend_class]
    struct ExtendClassItem
);

define_content_item!(
    /// #[pymember]
    struct MemberItem
);

struct ImplItemArgs<'a, Item: ItemLike> {
    item: &'a Item,
    attrs: &'a mut Vec<Attribute>,
    context: &'a mut ImplContext,
    cfgs: &'a [Attribute],
}

trait ImplItem<Item>: ContentItem
where
    Item: ItemLike + ToTokens + GetIdent,
{
    fn gen_impl_item(&self, args: ImplItemArgs<'_, Item>) -> Result<()>;
}

impl<Item> ImplItem<Item> for MethodItem
where
    Item: ItemLike + ToTokens + GetIdent,
{
    fn gen_impl_item(&self, args: ImplItemArgs<'_, Item>) -> Result<()> {
        let func = args
            .item
            .function_or_method()
            .map_err(|_| self.new_syn_error(args.item.span(), "can only be on a method"))?;
        let ident = &func.sig().ident;

        let item_attr = args.attrs.remove(self.index());
        let item_meta = MethodItemMeta::from_attr(ident.clone(), &item_attr)?;

        let py_name = item_meta.method_name()?;

        // Disallow slot methods - they should be defined via trait implementations
        // These are exposed as wrapper_descriptor via add_operators from SLOT_DEFS
        if !args.context.is_trait {
            const FORBIDDEN_SLOT_METHODS: &[(&str, &str)] = &[
                // Constructor/Initializer traits
                ("__new__", "Constructor"),
                ("__init__", "Initializer"),
                // Representable trait
                // ("__repr__", "Representable"),
                // ("__str__", "???"), // allow __str__
                // Hashable trait
                ("__hash__", "Hashable"),
                // Callable trait
                ("__call__", "Callable"),
                // GetAttr/SetAttr traits
                // NOTE: __getattribute__, __setattr__, __delattr__ are intentionally NOT forbidden
                // because they need pymethod for subclass override mechanism to work properly.
                // GetDescriptor/SetDescriptor traits
                // ("__get__", "GetDescriptor"),
                // ("__set__", "SetDescriptor"),
                // ("__delete__", "SetDescriptor"),
                // AsNumber trait
                ("__add__", "AsNumber"),
                ("__radd__", "AsNumber"),
                ("__iadd__", "AsNumber"),
                ("__sub__", "AsNumber"),
                ("__rsub__", "AsNumber"),
                ("__isub__", "AsNumber"),
                ("__mul__", "AsNumber"),
                ("__rmul__", "AsNumber"),
                ("__imul__", "AsNumber"),
                ("__truediv__", "AsNumber"),
                ("__rtruediv__", "AsNumber"),
                ("__itruediv__", "AsNumber"),
                ("__floordiv__", "AsNumber"),
                ("__rfloordiv__", "AsNumber"),
                ("__ifloordiv__", "AsNumber"),
                ("__mod__", "AsNumber"),
                ("__rmod__", "AsNumber"),
                ("__imod__", "AsNumber"),
                ("__pow__", "AsNumber"),
                ("__rpow__", "AsNumber"),
                ("__ipow__", "AsNumber"),
                ("__divmod__", "AsNumber"),
                ("__rdivmod__", "AsNumber"),
                ("__matmul__", "AsNumber"),
                ("__rmatmul__", "AsNumber"),
                ("__imatmul__", "AsNumber"),
                ("__lshift__", "AsNumber"),
                ("__rlshift__", "AsNumber"),
                ("__ilshift__", "AsNumber"),
                ("__rshift__", "AsNumber"),
                ("__rrshift__", "AsNumber"),
                ("__irshift__", "AsNumber"),
                ("__and__", "AsNumber"),
                ("__rand__", "AsNumber"),
                ("__iand__", "AsNumber"),
                ("__or__", "AsNumber"),
                ("__ror__", "AsNumber"),
                ("__ior__", "AsNumber"),
                ("__xor__", "AsNumber"),
                ("__rxor__", "AsNumber"),
                ("__ixor__", "AsNumber"),
                ("__neg__", "AsNumber"),
                ("__pos__", "AsNumber"),
                ("__abs__", "AsNumber"),
                ("__invert__", "AsNumber"),
                ("__int__", "AsNumber"),
                ("__float__", "AsNumber"),
                ("__index__", "AsNumber"),
                ("__bool__", "AsNumber"),
                // AsSequence trait
                // ("__len__", "AsSequence (or AsMapping)"),
                // ("__contains__", "AsSequence"),
                // AsMapping trait
                // ("__getitem__", "AsMapping (or AsSequence)"),
                // ("__setitem__", "AsMapping (or AsSequence)"),
                // ("__delitem__", "AsMapping (or AsSequence)"),
                // IterNext trait
                // ("__iter__", "IterNext"),
                // ("__next__", "IterNext"),
                // Comparable trait
                ("__eq__", "Comparable"),
                ("__ne__", "Comparable"),
                ("__lt__", "Comparable"),
                ("__le__", "Comparable"),
                ("__gt__", "Comparable"),
                ("__ge__", "Comparable"),
            ];

            if let Some((_, trait_name)) = FORBIDDEN_SLOT_METHODS
                .iter()
                .find(|(method, _)| *method == py_name.as_str())
            {
                return Err(syn::Error::new(
                    ident.span(),
                    format!(
                        "#[pymethod] cannot define '{py_name}'. Use `impl {trait_name} for ...` instead. \
                         Slot methods are exposed as wrapper_descriptor automatically.",
                    ),
                ));
            }
        }

        let raw = item_meta.raw()?;
        let sig_doc = text_signature(func.sig(), &py_name);

        // Add #[allow(non_snake_case)] for setter methods like set___name__
        let method_name = ident.to_string();
        if method_name.starts_with("set_") && method_name.contains("__") {
            let allow_attr: Attribute = parse_quote!(#[allow(non_snake_case)]);
            args.attrs.push(allow_attr);
        }

        let doc = args.attrs.doc().map(|doc| format_doc(&sig_doc, &doc));
        args.context.method_items.add_item(MethodNurseryItem {
            py_name,
            cfgs: args.cfgs.to_vec(),
            ident: ident.to_owned(),
            doc,
            raw,
            attr_name: self.inner.attr_name,
        });
        Ok(())
    }
}

impl<Item> ImplItem<Item> for GetSetItem
where
    Item: ItemLike + ToTokens + GetIdent,
{
    fn gen_impl_item(&self, args: ImplItemArgs<'_, Item>) -> Result<()> {
        let func = args
            .item
            .function_or_method()
            .map_err(|_| self.new_syn_error(args.item.span(), "can only be on a method"))?;
        let ident = &func.sig().ident;

        let item_attr = args.attrs.remove(self.index());
        let item_meta = GetSetItemMeta::from_attr(ident.clone(), &item_attr)?;

        let (py_name, kind) = item_meta.getset_name()?;

        // Add #[allow(non_snake_case)] for setter methods
        if matches!(kind, GetSetItemKind::Set) {
            let allow_attr: Attribute = parse_quote!(#[allow(non_snake_case)]);
            args.attrs.push(allow_attr);
        }

        args.context
            .getset_items
            .add_item(py_name, args.cfgs.to_vec(), kind, ident.clone())?;
        Ok(())
    }
}

impl<Item> ImplItem<Item> for SlotItem
where
    Item: ItemLike + ToTokens + GetIdent,
{
    fn gen_impl_item(&self, args: ImplItemArgs<'_, Item>) -> Result<()> {
        let (ident, span) = if let Ok(c) = args.item.constant() {
            (c.ident(), c.span())
        } else if let Ok(f) = args.item.function_or_method() {
            (&f.sig().ident, f.span())
        } else {
            return Err(self.new_syn_error(
                args.item.span(),
                "can only be on a method or const function pointer",
            ));
        };

        let item_attr = args.attrs.remove(self.index());
        let item_meta = SlotItemMeta::from_attr(ident.clone(), &item_attr)?;

        let slot_ident = item_meta.slot_name()?;
        let slot_ident = Ident::new(&slot_ident.to_string().to_lowercase(), slot_ident.span());
        let slot_name = slot_ident.to_string();
        let tokens = {
            const NON_ATOMIC_SLOTS: &[&str] = &["as_buffer"];
            const POINTER_SLOTS: &[&str] = &["as_sequence", "as_mapping"];
            const STATIC_GEN_SLOTS: &[&str] = &["as_number"];

            if NON_ATOMIC_SLOTS.contains(&slot_name.as_str()) {
                quote_spanned! { span =>
                    slots.#slot_ident = Some(Self::#ident as _);
                }
            } else if POINTER_SLOTS.contains(&slot_name.as_str()) {
                quote_spanned! { span =>
                    slots.#slot_ident.store(Some(PointerSlot::from(Self::#ident())));
                }
            } else if STATIC_GEN_SLOTS.contains(&slot_name.as_str()) {
                quote_spanned! { span =>
                    slots.#slot_ident = Self::#ident().into();
                }
            } else {
                quote_spanned! { span =>
                    slots.#slot_ident.store(Some(Self::#ident as _));
                }
            }
        };

        let pyname = format!("(slot {slot_name})");
        args.context.extend_slots_items.add_item(
            ident.clone(),
            vec![pyname],
            args.cfgs.to_vec(),
            tokens,
            2,
        )?;

        Ok(())
    }
}

impl<Item> ImplItem<Item> for AttributeItem
where
    Item: ItemLike + ToTokens + GetIdent,
{
    fn gen_impl_item(&self, args: ImplItemArgs<'_, Item>) -> Result<()> {
        let cfgs = args.cfgs.to_vec();
        let attr = args.attrs.remove(self.index());

        let get_py_name = |attr: &Attribute, ident: &Ident| -> Result<_> {
            let item_meta = SimpleItemMeta::from_attr(ident.clone(), attr)?;
            let py_name = item_meta.simple_name()?;
            Ok(py_name)
        };
        let (ident, py_name, tokens) =
            if args.item.function_or_method().is_ok() || args.item.constant().is_ok() {
                let ident = args.item.get_ident().unwrap();
                let py_name = get_py_name(&attr, ident)?;

                let value = if args.item.constant().is_ok() {
                    // TODO: ctx.new_value
                    quote_spanned!(ident.span() => ctx.new_int(Self::#ident).into())
                } else {
                    quote_spanned!(ident.span() => Self::#ident(ctx))
                };
                (
                    ident,
                    py_name.clone(),
                    quote! {
                        class.set_str_attr(#py_name, #value, ctx);
                    },
                )
            } else {
                return Err(self.new_syn_error(
                    args.item.span(),
                    "can only be on a const or an associated method without argument",
                ));
            };

        args.context
            .attribute_items
            .add_item(ident.clone(), vec![py_name], cfgs, tokens, 1)?;

        Ok(())
    }
}

impl<Item> ImplItem<Item> for ExtendClassItem
where
    Item: ItemLike + ToTokens + GetIdent,
{
    fn gen_impl_item(&self, args: ImplItemArgs<'_, Item>) -> Result<()> {
        args.attrs.remove(self.index());

        let ident = &args
            .item
            .function_or_method()
            .map_err(|_| self.new_syn_error(args.item.span(), "can only be on a method"))?
            .sig()
            .ident;

        args.context.class_extensions.push(quote! {
            Self::#ident(ctx, class);
        });

        Ok(())
    }
}

impl<Item> ImplItem<Item> for MemberItem
where
    Item: ItemLike + ToTokens + GetIdent,
{
    fn gen_impl_item(&self, args: ImplItemArgs<'_, Item>) -> Result<()> {
        let func = args
            .item
            .function_or_method()
            .map_err(|_| self.new_syn_error(args.item.span(), "can only be on a method"))?;
        let ident = &func.sig().ident;

        let item_attr = args.attrs.remove(self.index());
        let item_meta = MemberItemMeta::from_attr(ident.clone(), &item_attr)?;

        let (py_name, member_item_kind) = item_meta.member_name()?;
        let member_kind = match item_meta.member_kind()? {
            Some(s) => match s.as_str() {
                "bool" => MemberKind::Bool,
                _ => unreachable!(),
            },
            _ => MemberKind::ObjectEx,
        };

        // Add #[allow(non_snake_case)] for setter methods
        if matches!(member_item_kind, MemberItemKind::Set) {
            let allow_attr: Attribute = parse_quote!(#[allow(non_snake_case)]);
            args.attrs.push(allow_attr);
        }

        args.context.member_items.add_item(
            py_name,
            member_item_kind,
            member_kind,
            ident.clone(),
        )?;
        Ok(())
    }
}

#[derive(Default)]
struct MethodNursery {
    items: Vec<MethodNurseryItem>,
}

struct MethodNurseryItem {
    py_name: String,
    cfgs: Vec<Attribute>,
    ident: Ident,
    raw: bool,
    doc: Option<String>,
    attr_name: AttrName,
}

impl MethodNursery {
    fn add_item(&mut self, item: MethodNurseryItem) {
        self.items.push(item);
    }

    fn validate(&mut self) -> Result<()> {
        let mut name_set = HashSet::new();
        for item in &self.items {
            if !name_set.insert((&item.py_name, &item.cfgs)) {
                bail_span!(item.ident, "duplicate method name `{}`", item.py_name);
            }
        }
        Ok(())
    }
}

impl ToTokens for MethodNursery {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let mut inner_tokens = TokenStream::new();
        for item in &self.items {
            let py_name = &item.py_name;
            let ident = &item.ident;
            let cfgs = &item.cfgs;
            let doc = if let Some(doc) = item.doc.as_ref() {
                quote! { Some(#doc) }
            } else {
                quote! { None }
            };
            let flags = match &item.attr_name {
                AttrName::Method => {
                    quote! { rustpython_vm::function::PyMethodFlags::METHOD }
                }
                AttrName::ClassMethod => {
                    quote! { rustpython_vm::function::PyMethodFlags::CLASS }
                }
                AttrName::StaticMethod => {
                    quote! { rustpython_vm::function::PyMethodFlags::STATIC }
                }
                _ => unreachable!(),
            };
            // TODO: intern
            // let py_name = if py_name.starts_with("__") && py_name.ends_with("__") {
            //     let name_ident = Ident::new(&py_name, ident.span());
            //     quote_spanned! { ident.span() => ctx.names.#name_ident }
            // } else {
            //     quote_spanned! { ident.span() => #py_name }
            // };
            let method_new = if item.raw {
                quote!(new_raw_const)
            } else {
                quote!(new_const)
            };
            inner_tokens.extend(quote! [
                #(#cfgs)*
                rustpython_vm::function::PyMethodDef::#method_new(
                    #py_name,
                    Self::#ident,
                    #flags,
                    #doc,
                ),
            ]);
        }
        let array: TokenTree = Group::new(Delimiter::Bracket, inner_tokens).into();
        tokens.extend([array]);
    }
}

#[derive(Default)]
#[allow(clippy::type_complexity)]
struct GetSetNursery {
    map: HashMap<(String, Vec<Attribute>), (Option<Ident>, Option<Ident>)>,
    validated: bool,
}

enum GetSetItemKind {
    Get,
    Set,
}

impl GetSetNursery {
    fn add_item(
        &mut self,
        name: String,
        cfgs: Vec<Attribute>,
        kind: GetSetItemKind,
        item_ident: Ident,
    ) -> Result<()> {
        assert!(!self.validated, "new item is not allowed after validation");
        // Note: Both getter and setter can have #[cfg], but they must have matching cfgs
        // since the map key is (name, cfgs). This ensures getter and setter are paired correctly.
        let entry = self.map.entry((name.clone(), cfgs)).or_default();
        let func = match kind {
            GetSetItemKind::Get => &mut entry.0,
            GetSetItemKind::Set => &mut entry.1,
        };
        if func.is_some() {
            bail_span!(
                item_ident,
                "Multiple property accessors with name '{}'",
                name
            );
        }
        *func = Some(item_ident);
        Ok(())
    }

    fn validate(&mut self) -> Result<()> {
        let mut errors = Vec::new();
        for ((name, _cfgs), (getter, setter)) in &self.map {
            if getter.is_none() {
                errors.push(err_span!(
                    setter.as_ref().unwrap(),
                    "GetSet '{}' is missing a getter",
                    name
                ));
            };
        }
        errors.into_result()?;
        self.validated = true;
        Ok(())
    }
}

impl ToTokens for GetSetNursery {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        assert!(self.validated, "Call `validate()` before token generation");
        let properties = self.map.iter().map(|((name, cfgs), (getter, setter))| {
            let setter = match setter {
                Some(setter) => quote_spanned! { setter.span() => .with_set(Self::#setter)},
                None => quote! {},
            };
            quote_spanned! { getter.span() =>
                #( #cfgs )*
                class.set_str_attr(
                    #name,
                    ::rustpython_vm::PyRef::new_ref(
                        ::rustpython_vm::builtins::PyGetSet::new(#name.into(), class)
                            .with_get(Self::#getter)
                            #setter,
                            ctx.types.getset_type.to_owned(), None),
                    ctx
                );
            }
        });
        tokens.extend(properties);
    }
}

#[derive(Default)]
#[allow(clippy::type_complexity)]
struct MemberNursery {
    map: HashMap<(String, MemberKind), (Option<Ident>, Option<Ident>)>,
    validated: bool,
}

enum MemberItemKind {
    Get,
    Set,
}

#[derive(Eq, PartialEq, Hash)]
enum MemberKind {
    Bool,
    ObjectEx,
}

impl MemberNursery {
    fn add_item(
        &mut self,
        name: String,
        kind: MemberItemKind,
        member_kind: MemberKind,
        item_ident: Ident,
    ) -> Result<()> {
        assert!(!self.validated, "new item is not allowed after validation");
        let entry = self.map.entry((name.clone(), member_kind)).or_default();
        let func = match kind {
            MemberItemKind::Get => &mut entry.0,
            MemberItemKind::Set => &mut entry.1,
        };
        if func.is_some() {
            bail_span!(item_ident, "Multiple member accessors with name '{}'", name);
        }
        *func = Some(item_ident);
        Ok(())
    }

    fn validate(&mut self) -> Result<()> {
        let mut errors = Vec::new();
        for ((name, _), (getter, setter)) in &self.map {
            if getter.is_none() {
                errors.push(err_span!(
                    setter.as_ref().unwrap(),
                    "Member '{}' is missing a getter",
                    name
                ));
            };
        }
        errors.into_result()?;
        self.validated = true;
        Ok(())
    }
}

impl ToTokens for MemberNursery {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        assert!(self.validated, "Call `validate()` before token generation");
        let properties = self
            .map
            .iter()
            .map(|((name, member_kind), (getter, setter))| {
                let setter = match setter {
                    Some(setter) => quote_spanned! { setter.span() => Some(Self::#setter)},
                    None => quote! { None },
                };
                let member_kind = match member_kind {
                    MemberKind::Bool => {
                        quote!(::rustpython_vm::builtins::descriptor::MemberKind::Bool)
                    }
                    MemberKind::ObjectEx => {
                        quote!(::rustpython_vm::builtins::descriptor::MemberKind::ObjectEx)
                    }
                };
                quote_spanned! { getter.span() =>
                    class.set_str_attr(
                        #name,
                        ctx.new_member(#name, #member_kind, Self::#getter, #setter, class),
                        ctx,
                    );
                }
            });
        tokens.extend(properties);
    }
}

struct MethodItemMeta(ItemMetaInner);

impl ItemMeta for MethodItemMeta {
    const ALLOWED_NAMES: &'static [&'static str] = &["name", "raw"];

    fn from_inner(inner: ItemMetaInner) -> Self {
        Self(inner)
    }

    fn inner(&self) -> &ItemMetaInner {
        &self.0
    }
}

impl MethodItemMeta {
    fn raw(&self) -> Result<bool> {
        self.inner()._bool("raw")
    }

    fn method_name(&self) -> Result<String> {
        let inner = self.inner();
        let name = inner._optional_str("name")?;
        Ok(if let Some(name) = name {
            name
        } else {
            inner.item_name()
        })
    }
}

struct GetSetItemMeta(ItemMetaInner);

impl ItemMeta for GetSetItemMeta {
    const ALLOWED_NAMES: &'static [&'static str] = &["name", "setter"];

    fn from_inner(inner: ItemMetaInner) -> Self {
        Self(inner)
    }

    fn inner(&self) -> &ItemMetaInner {
        &self.0
    }
}

impl GetSetItemMeta {
    fn getset_name(&self) -> Result<(String, GetSetItemKind)> {
        let inner = self.inner();
        let kind = if inner._bool("setter")? {
            GetSetItemKind::Set
        } else {
            GetSetItemKind::Get
        };
        let name = inner._optional_str("name")?;
        let py_name = if let Some(name) = name {
            name
        } else {
            let sig_name = inner.item_name();
            let extract_prefix_name = |prefix, item_typ| {
                if let Some(name) = sig_name.strip_prefix(prefix) {
                    if name.is_empty() {
                        Err(err_span!(
                            inner.meta_ident,
                            r#"A #[{}({typ})] fn with a {prefix}* name must \
                             have something after "{prefix}""#,
                            inner.meta_name(),
                            typ = item_typ,
                            prefix = prefix
                        ))
                    } else {
                        Ok(name.to_owned())
                    }
                } else {
                    Err(err_span!(
                        inner.meta_ident,
                        r#"A #[{}(setter)] fn must either have a `name` \
                         parameter or a fn name along the lines of "set_*""#,
                        inner.meta_name()
                    ))
                }
            };
            match kind {
                GetSetItemKind::Get => sig_name,
                GetSetItemKind::Set => extract_prefix_name("set_", "setter")?,
            }
        };
        Ok((py_name, kind))
    }
}

struct SlotItemMeta(ItemMetaInner);

impl ItemMeta for SlotItemMeta {
    const ALLOWED_NAMES: &'static [&'static str] = &[]; // not used

    fn from_nested<I>(item_ident: Ident, meta_ident: Ident, mut nested: I) -> Result<Self>
    where
        I: core::iter::Iterator<Item = NestedMeta>,
    {
        let meta_map = if let Some(nested_meta) = nested.next() {
            match nested_meta {
                NestedMeta::Meta(meta) => {
                    Some([("name".to_owned(), (0, meta))].iter().cloned().collect())
                }
                _ => None,
            }
        } else {
            Some(HashMap::default())
        };
        let (Some(meta_map), None) = (meta_map, nested.next()) else {
            bail_span!(
                meta_ident,
                "#[pyslot] must be of the form #[pyslot] or #[pyslot(slot_name)]"
            )
        };
        Ok(Self::from_inner(ItemMetaInner {
            item_ident,
            meta_ident,
            meta_map,
        }))
    }

    fn from_inner(inner: ItemMetaInner) -> Self {
        Self(inner)
    }

    fn inner(&self) -> &ItemMetaInner {
        &self.0
    }
}

impl SlotItemMeta {
    fn slot_name(&self) -> Result<Ident> {
        let inner = self.inner();
        let method_name = if let Some((_, meta)) = inner.meta_map.get("name") {
            match meta {
                Meta::Path(path) => path.get_ident().cloned(),
                _ => None,
            }
        } else {
            let ident_str = self.inner().item_name();
            // Convert to lowercase to handle both SLOT_NEW and slot_new
            let ident_lower = ident_str.to_lowercase();
            let name = if let Some(stripped) = ident_lower.strip_prefix("slot_") {
                proc_macro2::Ident::new(stripped, inner.item_ident.span())
            } else {
                inner.item_ident.clone()
            };
            Some(name)
        };
        let method_name = method_name.ok_or_else(|| {
            err_span!(
                inner.meta_ident,
                "#[pyslot] must be of the form #[pyslot] or #[pyslot(slot_name)]",
            )
        })?;

        // Strip double underscores from slot names like __init__ -> init
        let method_name_str = method_name.to_string();
        let slot_name = if method_name_str.starts_with("__")
            && method_name_str.ends_with("__")
            && method_name_str.len() > 4
        {
            &method_name_str[2..method_name_str.len() - 2]
        } else {
            &method_name_str
        };

        Ok(proc_macro2::Ident::new(slot_name, slot_name.span()))
    }
}

struct MemberItemMeta(ItemMetaInner);

impl ItemMeta for MemberItemMeta {
    const ALLOWED_NAMES: &'static [&'static str] = &["type", "setter"];

    fn from_inner(inner: ItemMetaInner) -> Self {
        Self(inner)
    }

    fn inner(&self) -> &ItemMetaInner {
        &self.0
    }
}

impl MemberItemMeta {
    fn member_name(&self) -> Result<(String, MemberItemKind)> {
        let inner = self.inner();
        let sig_name = inner.item_name();
        let extract_prefix_name = |prefix, item_typ| {
            if let Some(name) = sig_name.strip_prefix(prefix) {
                if name.is_empty() {
                    Err(err_span!(
                        inner.meta_ident,
                        r#"A #[{}({typ})] fn with a {prefix}* name must \
                         have something after "{prefix}""#,
                        inner.meta_name(),
                        typ = item_typ,
                        prefix = prefix
                    ))
                } else {
                    Ok(name.to_owned())
                }
            } else {
                Err(err_span!(
                    inner.meta_ident,
                    r#"A #[{}(setter)] fn must either have a `name` \
                     parameter or a fn name along the lines of "set_*""#,
                    inner.meta_name()
                ))
            }
        };
        let kind = if inner._bool("setter")? {
            MemberItemKind::Set
        } else {
            MemberItemKind::Get
        };
        let name = match kind {
            MemberItemKind::Get => sig_name,
            MemberItemKind::Set => extract_prefix_name("set_", "setter")?,
        };
        Ok((name, kind))
    }

    fn member_kind(&self) -> Result<Option<String>> {
        let inner = self.inner();
        inner._optional_str("type")
    }
}

struct ExtractedImplAttrs {
    payload: Option<Ident>,
    flags: TokenStream,
    with_impl: TokenStream,
    with_method_defs: Vec<TokenStream>,
    with_slots: TokenStream,
    itemsize: Option<syn::Expr>,
}

fn extract_impl_attrs(attr: PunctuatedNestedMeta, item: &Ident) -> Result<ExtractedImplAttrs> {
    let mut withs = Vec::new();
    let mut with_method_defs = Vec::new();
    let mut with_slots = Vec::new();
    let mut flags = vec![quote! {
        {
            #[cfg(not(debug_assertions))] {
                ::rustpython_vm::types::PyTypeFlags::DEFAULT
            }
            #[cfg(debug_assertions)] {
                ::rustpython_vm::types::PyTypeFlags::DEFAULT
                    .union(::rustpython_vm::types::PyTypeFlags::_CREATED_WITH_FLAGS)
            }
        }
    }];
    let mut payload = None;
    let mut itemsize = None;

    for attr in attr {
        match attr {
            NestedMeta::Meta(Meta::List(MetaList { path, nested, .. })) => {
                if path.is_ident("with") {
                    for meta in nested {
                        let NestedMeta::Meta(Meta::Path(path)) = &meta else {
                            bail_span!(meta, "#[pyclass(with(...))] arguments must be paths")
                        };
                        let (extend_class, method_defs, extend_slots) = if path.is_ident("PyRef")
                            || path.is_ident("Py")
                        {
                            // special handling for PyRef
                            (
                                quote!(#path::<Self>::__extend_py_class),
                                quote!(#path::<Self>::__OWN_METHOD_DEFS),
                                quote!(#path::<Self>::__extend_slots),
                            )
                        } else {
                            if path.is_ident("DefaultConstructor") {
                                bail_span!(
                                    meta,
                                    "Try `#[pyclass(with(Constructor, ...))]` instead of `#[pyclass(with(DefaultConstructor, ...))]`. DefaultConstructor implicitly implements Constructor."
                                )
                            }
                            (
                                quote!(<Self as #path>::__extend_py_class),
                                quote!(<Self as #path>::__OWN_METHOD_DEFS),
                                quote!(<Self as #path>::__extend_slots),
                            )
                        };
                        let item_span = item.span().resolved_at(Span::call_site());
                        withs.push(quote_spanned! { path.span() =>
                            #extend_class(ctx, class);
                        });
                        with_method_defs.push(method_defs);
                        // For Initializer and Constructor traits, directly set the slot
                        // instead of calling __extend_slots. This ensures that the trait
                        // impl's override (e.g., slot_init in impl Initializer) is used,
                        // not the trait's default implementation.
                        let slot_code = if path.is_ident("Initializer") {
                            quote_spanned! { item_span =>
                                slots.init.store(Some(<Self as ::rustpython_vm::types::Initializer>::slot_init as _));
                            }
                        } else if path.is_ident("Constructor") {
                            quote_spanned! { item_span =>
                                slots.new.store(Some(<Self as ::rustpython_vm::types::Constructor>::slot_new as _));
                            }
                        } else {
                            quote_spanned! { item_span =>
                                #extend_slots(slots);
                            }
                        };
                        with_slots.push(slot_code);
                    }
                } else if path.is_ident("flags") {
                    for meta in nested {
                        let NestedMeta::Meta(Meta::Path(path)) = meta else {
                            bail_span!(meta, "#[pyclass(flags(...))] arguments should be ident")
                        };
                        let ident = path.get_ident().ok_or_else(|| {
                            err_span!(path, "#[pyclass(flags(...))] arguments should be ident")
                        })?;
                        flags.push(quote_spanned! { ident.span() =>
                             .union(::rustpython_vm::types::PyTypeFlags::#ident)
                        });
                    }
                    flags.push(quote! {
                        .union(::rustpython_vm::types::PyTypeFlags::IMMUTABLETYPE)
                    });
                } else {
                    bail_span!(path, "Unknown pyimpl attribute")
                }
            }
            NestedMeta::Meta(Meta::NameValue(syn::MetaNameValue { path, value, .. })) => {
                if path.is_ident("payload") {
                    if let syn::Expr::Lit(syn::ExprLit {
                        lit: syn::Lit::Str(lit),
                        ..
                    }) = value
                    {
                        payload = Some(Ident::new(&lit.value(), lit.span()));
                    } else {
                        bail_span!(value, "payload must be a string literal")
                    }
                } else if path.is_ident("itemsize") {
                    itemsize = Some(value);
                } else {
                    bail_span!(path, "Unknown pyimpl attribute")
                }
            }
            attr => bail_span!(attr, "Unknown pyimpl attribute"),
        }
    }

    Ok(ExtractedImplAttrs {
        payload,
        flags: quote! {
            #(#flags)*
        },
        with_impl: quote! {
            #(#withs)*
        },
        with_method_defs,
        with_slots: quote! {
            #(#with_slots)*
        },
        itemsize,
    })
}

fn impl_item_new<Item>(
    index: usize,
    attr_name: AttrName,
) -> Result<Box<dyn ImplItem<Item, AttrName = AttrName>>>
where
    Item: ItemLike + ToTokens + GetIdent,
{
    use AttrName::*;
    Ok(match attr_name {
        attr_name @ Method | attr_name @ ClassMethod | attr_name @ StaticMethod => {
            Box::new(MethodItem {
                inner: ContentItemInner { index, attr_name },
            })
        }
        GetSet => Box::new(GetSetItem {
            inner: ContentItemInner { index, attr_name },
        }),
        Slot => Box::new(SlotItem {
            inner: ContentItemInner { index, attr_name },
        }),
        Attr => Box::new(AttributeItem {
            inner: ContentItemInner { index, attr_name },
        }),
        ExtendClass => Box::new(ExtendClassItem {
            inner: ContentItemInner { index, attr_name },
        }),
        Member => Box::new(MemberItem {
            inner: ContentItemInner { index, attr_name },
        }),
    })
}

fn attrs_to_content_items<F, R>(
    attrs: &[Attribute],
    item_new: F,
) -> Result<(Vec<R>, Vec<Attribute>)>
where
    F: Fn(usize, AttrName) -> Result<R>,
{
    let mut cfgs: Vec<Attribute> = Vec::new();
    let mut result = Vec::new();

    let mut iter = attrs.iter().enumerate().peekable();
    while let Some((_, attr)) = iter.peek() {
        // take all cfgs but no py items
        let attr = *attr;
        let attr_name = if let Some(ident) = attr.get_ident() {
            ident.to_string()
        } else {
            continue;
        };
        if attr_name == "cfg" {
            cfgs.push(attr.clone());
        } else if ALL_ALLOWED_NAMES.contains(&attr_name.as_str()) {
            break;
        }
        iter.next();
    }

    for (i, attr) in iter {
        // take py items but no cfgs
        let attr_name = if let Some(ident) = attr.get_ident() {
            ident.to_string()
        } else {
            continue;
        };
        if attr_name == "cfg" {
            bail_span!(attr, "#[py*] items must be placed under `cfgs`",);
        }
        let attr_name = match AttrName::from_str(attr_name.as_str()) {
            Ok(name) => name,
            Err(wrong_name) => {
                if ALL_ALLOWED_NAMES.contains(&attr_name.as_str()) {
                    bail_span!(attr, "#[pyclass] doesn't accept #[{}]", wrong_name)
                } else {
                    continue;
                }
            }
        };

        result.push(item_new(i, attr_name)?);
    }
    Ok((result, cfgs))
}

#[allow(dead_code)]
fn parse_vec_ident(
    attr: &[NestedMeta],
    item: &Item,
    index: usize,
    message: &str,
) -> Result<String> {
    Ok(attr
        .get(index)
        .ok_or_else(|| err_span!(item, "We require {} argument to be set", message))?
        .get_ident()
        .ok_or_else(|| {
            err_span!(
                item,
                "We require {} argument to be ident or string",
                message
            )
        })?
        .to_string())
}
