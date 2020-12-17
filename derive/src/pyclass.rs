use super::Diagnostic;
use crate::util::{
    path_eq, pyclass_ident_and_attrs, ClassItemMeta, ContentItem, ContentItemInner, ErrorVec,
    ItemMeta, ItemMetaInner, ItemNursery, SimpleItemMeta, ALL_ALLOWED_NAMES,
};
use proc_macro2::TokenStream;
use quote::{quote, quote_spanned, ToTokens};
use std::collections::HashMap;
use syn::{
    parse_quote, spanned::Spanned, Attribute, AttributeArgs, Ident, Item, Meta, NestedMeta, Result,
};
use syn_ext::ext::*;

#[derive(Default)]
struct ImplContext {
    impl_extend_items: ItemNursery,
    getset_items: GetSetNursery,
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
            let (pyitems, cfgs) = attrs_to_content_items(&attrs, new_impl_item::<Item>)?;
            for pyitem in pyitems.iter().rev() {
                let r = pyitem.gen_impl_item(ImplItemArgs::<Item> {
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
    context.errors.ok_or_push(context.getset_items.validate());
}

pub(crate) fn impl_pyimpl(
    attr: AttributeArgs,
    item: Item,
) -> std::result::Result<TokenStream, Diagnostic> {
    let mut context = ImplContext::default();
    let mut tokens = match item {
        Item::Impl(mut imp) => {
            extract_items_into_context(&mut context, imp.items.iter_mut());

            let ExtractedImplAttrs {
                with_impl,
                flags,
                with_slots,
            } = extract_impl_attrs(attr)?;

            let ty = &imp.self_ty;

            let getset_impl = &context.getset_items;
            let extend_impl = &context.impl_extend_items;
            let slots_impl = &context.extend_slots_items;
            let class_extensions = &context.class_extensions;
            quote! {
                #imp
                impl ::rustpython_vm::pyobject::PyClassImpl for #ty {
                    const TP_FLAGS: ::rustpython_vm::slots::PyTpFlags = ::rustpython_vm::slots::PyTpFlags::from_bits_truncate(#flags);

                    fn impl_extend_class(
                        ctx: &::rustpython_vm::pyobject::PyContext,
                        class: &::rustpython_vm::builtins::PyTypeRef,
                    ) {
                        #getset_impl
                        #extend_impl
                        #with_impl
                        #(#class_extensions)*
                    }

                    fn extend_slots(slots: &mut ::rustpython_vm::slots::PyTypeSlots) {
                        #with_slots
                        #slots_impl
                    }
                }
            }
        }
        Item::Trait(mut trai) => {
            let mut context = ImplContext::default();
            extract_items_into_context(&mut context, trai.items.iter_mut());

            let ExtractedImplAttrs {
                with_impl,
                with_slots,
                ..
            } = extract_impl_attrs(attr)?;

            let getset_impl = &context.getset_items;
            let extend_impl = &context.impl_extend_items;
            let slots_impl = &context.extend_slots_items;
            let class_extensions = &context.class_extensions;
            let extra_methods = iter_chain![
                parse_quote! {
                    fn __extend_py_class(
                        ctx: &::rustpython_vm::pyobject::PyContext,
                        class: &::rustpython_vm::builtins::PyTypeRef,
                    ) {
                        #getset_impl
                        #extend_impl
                        #with_impl
                        #(#class_extensions)*
                    }
                },
                parse_quote! {
                    fn __extend_slots(slots: &mut ::rustpython_vm::slots::PyTypeSlots) {
                        #with_slots
                        #slots_impl
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

fn generate_class_def(
    ident: &Ident,
    name: &str,
    module_name: Option<&str>,
    base: Option<String>,
    attrs: &[Attribute],
) -> std::result::Result<TokenStream, Diagnostic> {
    let doc = if let Some(doc) = attrs.doc() {
        quote!(Some(#doc))
    } else {
        quote!(None)
    };
    let module_class_name = if let Some(module_name) = module_name {
        format!("{}.{}", module_name, name)
    } else {
        name.to_owned()
    };
    let module_name = match module_name {
        Some(v) => quote!(Some(#v) ),
        None => quote!(None),
    };
    let is_pystruct = attrs.iter().any(|attr| {
        path_eq(&attr.path, "derive")
            && if let Ok(Meta::List(l)) = attr.parse_meta() {
                l.nested
                    .into_iter()
                    .any(|n| n.get_ident().map_or(false, |p| p == "PyStructSequence"))
            } else {
                false
            }
    });
    if base.is_some() && is_pystruct {
        return Err(syn::Error::new_spanned(
            ident,
            "PyStructSequence cannot have `base` class attr",
        )
        .into());
    }
    let base = base.map(|name| Ident::new(&name, ident.span()));

    let base_class = if is_pystruct {
        quote! {
            fn static_baseclass() -> &'static ::rustpython_vm::builtins::PyTypeRef {
                use rustpython_vm::pyobject::StaticType;
                rustpython_vm::builtins::PyTuple::static_type()
            }
        }
    } else if let Some(base) = base {
        quote! {
            fn static_baseclass() -> &'static ::rustpython_vm::builtins::PyTypeRef {
                use rustpython_vm::pyobject::StaticType;
                #base::static_type()
            }
        }
    } else {
        quote!()
    };

    let tokens = quote! {
        impl ::rustpython_vm::pyobject::PyClassDef for #ident {
            const NAME: &'static str = #name;
            const MODULE_NAME: Option<&'static str> = #module_name;
            const TP_NAME: &'static str = #module_class_name;
            const DOC: Option<&'static str> = #doc;
        }

        impl ::rustpython_vm::pyobject::StaticType for #ident {
            fn static_cell() -> &'static ::rustpython_common::static_cell::StaticCell<::rustpython_vm::builtins::PyTypeRef> {
                ::rustpython_common::static_cell! {
                    static CELL: ::rustpython_vm::builtins::PyTypeRef;
                }
                &CELL
            }

            #base_class
        }
    };
    Ok(tokens)
}

pub(crate) fn impl_pyclass(
    attr: AttributeArgs,
    item: Item,
) -> std::result::Result<TokenStream, Diagnostic> {
    let (ident, attrs) = pyclass_ident_and_attrs(&item)?;
    let fake_ident = Ident::new("pyclass", item.span());
    let class_meta = ClassItemMeta::from_nested(ident.clone(), fake_ident, attr.into_iter())?;
    let class_name = class_meta.class_name()?;
    let module_name = class_meta.module()?;
    let base = class_meta.base()?;
    let class_def = generate_class_def(&ident, &class_name, module_name.as_deref(), base, &attrs)?;

    let ret = quote! {
        #item
        #class_def
    };
    Ok(ret)
}

/// #[pymethod] and #[pyclassmethod]
struct MethodItem {
    inner: ContentItemInner,
    method_type: String,
}

/// #[pyproperty]
struct PropertyItem {
    inner: ContentItemInner,
}

/// #[pyslot]
struct SlotItem {
    inner: ContentItemInner,
}

/// #[pyattr]
struct AttributeItem {
    inner: ContentItemInner,
}

/// #[extend_class]
struct ExtendClassItem {
    inner: ContentItemInner,
}

impl ContentItem for MethodItem {
    fn inner(&self) -> &ContentItemInner {
        &self.inner
    }
}
impl ContentItem for PropertyItem {
    fn inner(&self) -> &ContentItemInner {
        &self.inner
    }
}
impl ContentItem for SlotItem {
    fn inner(&self) -> &ContentItemInner {
        &self.inner
    }
}
impl ContentItem for AttributeItem {
    fn inner(&self) -> &ContentItemInner {
        &self.inner
    }
}
impl ContentItem for ExtendClassItem {
    fn inner(&self) -> &ContentItemInner {
        &self.inner
    }
}

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
        let ident = if args.item.is_function_or_method() {
            Ok(args.item.get_ident().unwrap())
        } else {
            Err(self.new_syn_error(args.item.span(), "can only be on a method"))
        }?;

        let item_attr = args.attrs.remove(self.index());
        let item_meta = MethodItemMeta::from_attr(ident.clone(), &item_attr)?;

        let py_name = item_meta.method_name()?;
        let build_func = Ident::new(&format!("build_{}", &self.method_type), args.item.span());
        let tokens = {
            let doc = args.attrs.doc().map_or_else(
                TokenStream::new,
                |doc| quote!(.with_doc(#doc.to_owned(), ctx)),
            );
            quote! {
                class.set_str_attr(
                    #py_name,
                    ctx.make_funcdef(#py_name, Self::#ident)
                        #doc
                        .#build_func(ctx),
                );
            }
        };

        args.context
            .impl_extend_items
            .add_item(py_name, args.cfgs.to_vec(), tokens)?;
        Ok(())
    }
}

impl<Item> ImplItem<Item> for PropertyItem
where
    Item: ItemLike + ToTokens + GetIdent,
{
    fn gen_impl_item(&self, args: ImplItemArgs<'_, Item>) -> Result<()> {
        let ident = if args.item.is_function_or_method() {
            Ok(args.item.get_ident().unwrap())
        } else {
            Err(self.new_syn_error(args.item.span(), "can only be on a method"))
        }?;

        let item_attr = args.attrs.remove(self.index());
        let item_meta = PropertyItemMeta::from_attr(ident.clone(), &item_attr)?;

        let (py_name, kind) = item_meta.property_name()?;
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
        let ident = if args.item.is_function_or_method() {
            Ok(args.item.get_ident().unwrap())
        } else {
            Err(self.new_syn_error(args.item.span(), "can only be on a method"))
        }?;

        let item_attr = args.attrs.remove(self.index());
        let item_meta = SlotItemMeta::from_attr(ident.clone(), &item_attr)?;

        let slot_ident = item_meta.slot_name()?;
        let slot_name = slot_ident.to_string();
        let tokens = {
            let into_func = if slot_name == "new" {
                quote_spanned! {ident.span() =>
                    ::rustpython_vm::function::IntoPyNativeFunc::into_func(Self::#ident)
                }
            } else {
                quote_spanned! {ident.span() =>
                    Self::#ident as _
                }
            };
            if slot_name == "new" || slot_name == "buffer" {
                quote! {
                    slots.#slot_ident = Some(#into_func);
                }
            } else {
                quote! {
                    slots.#slot_ident.store(Some(#into_func))
                }
            }
        };

        args.context.extend_slots_items.add_item(
            format!("(slot {})", slot_name),
            args.cfgs.to_vec(),
            tokens,
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
        let (py_name, tokens) = if args.item.is_function_or_method() || args.item.is_const() {
            let ident = args.item.get_ident().unwrap();
            let py_name = get_py_name(&attr, &ident)?;

            let value = if args.item.is_const() {
                // TODO: ctx.new_value
                quote_spanned!(ident.span() => ctx.new_int(Self::#ident))
            } else {
                quote_spanned!(ident.span() => Self::#ident(ctx))
            };
            (
                py_name.clone(),
                quote! {
                    class.set_str_attr(#py_name, #value);
                },
            )
        } else {
            return Err(self.new_syn_error(
                args.item.span(),
                "can only be on a const or an associated method without argument",
            ));
        };

        args.context
            .impl_extend_items
            .add_item(py_name, cfgs, tokens)?;

        Ok(())
    }
}

impl<Item> ImplItem<Item> for ExtendClassItem
where
    Item: ItemLike + ToTokens + GetIdent,
{
    fn gen_impl_item(&self, args: ImplItemArgs<'_, Item>) -> Result<()> {
        args.attrs.remove(self.index());

        let ident = if args.item.is_function_or_method() {
            Ok(args.item.get_ident().unwrap())
        } else {
            Err(self.new_syn_error(args.item.span(), "can only be on a method"))
        }?;

        args.context.class_extensions.push(quote! {
            Self::#ident(ctx, class);
        });

        Ok(())
    }
}

#[derive(Default)]
#[allow(clippy::type_complexity)]
struct GetSetNursery {
    map: HashMap<(String, Vec<Attribute>), (Option<Ident>, Option<Ident>, Option<Ident>)>,
    validated: bool,
}

enum GetSetItemKind {
    Get,
    Set,
    Delete,
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
        if !matches!(kind, GetSetItemKind::Get) && !cfgs.is_empty() {
            return Err(syn::Error::new_spanned(
                item_ident,
                "Only the getter can have #[cfg]",
            ));
        }
        let entry = self.map.entry((name.clone(), cfgs)).or_default();
        let func = match kind {
            GetSetItemKind::Get => &mut entry.0,
            GetSetItemKind::Set => &mut entry.1,
            GetSetItemKind::Delete => &mut entry.2,
        };
        if func.is_some() {
            return Err(syn::Error::new_spanned(
                item_ident,
                format!("Multiple property accessors with name '{}'", name),
            ));
        }
        *func = Some(item_ident);
        Ok(())
    }

    fn validate(&mut self) -> Result<()> {
        let mut errors = Vec::new();
        for ((name, _cfgs), (getter, setter, deleter)) in self.map.iter() {
            if getter.is_none() {
                errors.push(syn::Error::new_spanned(
                    setter.as_ref().or_else(|| deleter.as_ref()).unwrap(),
                    format!("Property '{}' is missing a getter", name),
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
        let properties = self
            .map
            .iter()
            .map(|((name, cfgs), (getter, setter, deleter))| {
                let setter = match setter {
                    Some(setter) => quote_spanned! { setter.span()=> .with_set(&Self::#setter)},
                    None => quote! {},
                };
                let deleter = match deleter {
                    Some(deleter) => {
                        quote_spanned! { deleter.span()=> .with_delete(&Self::#deleter)}
                    }
                    None => quote! {},
                };
                quote! {
                    #( #cfgs )*
                    class.set_str_attr(
                        #name,
                        ::rustpython_vm::pyobject::PyObject::new(
                            ::rustpython_vm::builtins::PyGetSet::new(#name.into())
                                .with_get(&Self::#getter)
                                #setter #deleter,
                            ctx.types.getset_type.clone(), None)
                    );
                }
            });
        tokens.extend(properties);
    }
}

struct MethodItemMeta(ItemMetaInner);

impl ItemMeta for MethodItemMeta {
    const ALLOWED_NAMES: &'static [&'static str] = &["name", "magic"];

    fn from_inner(inner: ItemMetaInner) -> Self {
        Self(inner)
    }
    fn inner(&self) -> &ItemMetaInner {
        &self.0
    }
}

impl MethodItemMeta {
    fn method_name(&self) -> Result<String> {
        let inner = self.inner();
        let name = inner._optional_str("name")?;
        let magic = inner._bool("magic")?;
        Ok(if let Some(name) = name {
            name
        } else {
            let name = inner.item_name();
            if magic {
                format!("__{}__", name)
            } else {
                name
            }
        })
    }
}

struct PropertyItemMeta(ItemMetaInner);

impl ItemMeta for PropertyItemMeta {
    const ALLOWED_NAMES: &'static [&'static str] = &["name", "magic", "setter", "deleter"];

    fn from_inner(inner: ItemMetaInner) -> Self {
        Self(inner)
    }
    fn inner(&self) -> &ItemMetaInner {
        &self.0
    }
}

impl PropertyItemMeta {
    fn property_name(&self) -> Result<(String, GetSetItemKind)> {
        let inner = self.inner();
        let magic = inner._bool("magic")?;
        let kind = match (inner._bool("setter")?, inner._bool("deleter")?) {
            (false, false) => GetSetItemKind::Get,
            (true, false) => GetSetItemKind::Set,
            (false, true) => GetSetItemKind::Delete,
            (true, true) => {
                return Err(syn::Error::new_spanned(
                    &inner.meta_ident,
                    format!(
                        "can't have both setter and deleter on a #[{}] fn",
                        inner.meta_name()
                    ),
                ))
            }
        };
        let name = inner._optional_str("name")?;
        let py_name = if let Some(name) = name {
            name
        } else {
            let sig_name = inner.item_name();
            let extract_prefix_name = |prefix, item_typ| {
                if let Some(name) = sig_name.strip_prefix(prefix) {
                    if name.is_empty() {
                        Err(syn::Error::new_spanned(
                            &inner.meta_ident,
                            format!(
                                "A #[{}({typ})] fn with a {prefix}* name must \
                                 have something after \"{prefix}\"",
                                inner.meta_name(),
                                typ = item_typ,
                                prefix = prefix
                            ),
                        ))
                    } else {
                        Ok(name.to_owned())
                    }
                } else {
                    Err(syn::Error::new_spanned(
                        &inner.meta_ident,
                        format!(
                            "A #[{}(setter)] fn must either have a `name` \
                             parameter or a fn name along the lines of \"set_*\"",
                            inner.meta_name()
                        ),
                    ))
                }
            };
            let name = match kind {
                GetSetItemKind::Get => sig_name,
                GetSetItemKind::Set => extract_prefix_name("set_", "setter")?,
                GetSetItemKind::Delete => extract_prefix_name("del_", "deleter")?,
            };
            if magic {
                format!("__{}__", name)
            } else {
                name
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
        I: std::iter::Iterator<Item = NestedMeta>,
    {
        let meta_map = if let Some(nmeta) = nested.next() {
            match nmeta {
                NestedMeta::Meta(meta) => {
                    Some([("name".to_owned(), (0, meta))].iter().cloned().collect())
                }
                _ => None,
            }
        } else {
            Some(HashMap::default())
        };
        match (meta_map, nested.next()) {
            (Some(meta_map), None) => Ok(Self::from_inner(ItemMetaInner {
                item_ident,
                meta_ident,
                meta_map,
            })),
            _ => Err(syn::Error::new_spanned(
                meta_ident,
                "#[pyslot] must be of the form #[pyslot] or #[pyslot(slotname)]",
            )),
        }
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
        let slot_name = if let Some((_, meta)) = inner.meta_map.get("name") {
            match meta {
                Meta::Path(path) => path.get_ident().cloned(),
                _ => None,
            }
        } else {
            let ident_str = self.inner().item_name();
            let name = if let Some(stripped) = ident_str.strip_prefix("tp_") {
                proc_macro2::Ident::new(stripped, inner.item_ident.span())
            } else {
                inner.item_ident.clone()
            };
            Some(name)
        };
        slot_name.ok_or_else(|| {
            syn::Error::new_spanned(
                &inner.meta_ident,
                "#[pyslot] must be of the form #[pyslot] or #[pyslot(slotname)]",
            )
        })
    }
}

struct ExtractedImplAttrs {
    with_impl: TokenStream,
    with_slots: TokenStream,
    flags: TokenStream,
}

fn extract_impl_attrs(attr: AttributeArgs) -> std::result::Result<ExtractedImplAttrs, Diagnostic> {
    let mut withs = Vec::new();
    let mut with_slots = Vec::new();
    let mut flags = vec![quote! { ::rustpython_vm::slots::PyTpFlags::DEFAULT.bits() }];
    #[cfg(debug_assertions)]
    {
        flags.push(quote! {
            | ::rustpython_vm::slots::PyTpFlags::_CREATED_WITH_FLAGS.bits()
        });
    }

    for attr in attr {
        match attr {
            NestedMeta::Meta(Meta::List(syn::MetaList { path, nested, .. })) => {
                if path_eq(&path, "with") {
                    for meta in nested {
                        let path = match meta {
                            NestedMeta::Meta(Meta::Path(path)) => path,
                            meta => {
                                bail_span!(meta, "#[pyimpl(with(...))] arguments should be paths")
                            }
                        };
                        if path_eq(&path, "PyRef") {
                            // special handling for PyRef
                            withs.push(quote! {
                                PyRef::<Self>::impl_extend_class(ctx, class);
                            });
                            with_slots.push(quote! {
                                PyRef::<Self>::extend_slots(slots);
                            });
                        } else {
                            withs.push(quote! {
                                <Self as #path>::__extend_py_class(ctx, class);
                            });
                            with_slots.push(quote! {
                                <Self as #path>::__extend_slots(slots);
                            });
                        }
                    }
                } else if path_eq(&path, "flags") {
                    for meta in nested {
                        match meta {
                            NestedMeta::Meta(Meta::Path(path)) => {
                                if let Some(ident) = path.get_ident() {
                                    flags.push(quote! {
                                        | ::rustpython_vm::slots::PyTpFlags::#ident.bits()
                                    });
                                } else {
                                    bail_span!(
                                        path,
                                        "#[pyimpl(flags(...))] arguments should be ident"
                                    )
                                }
                            }
                            meta => {
                                bail_span!(meta, "#[pyimpl(flags(...))] arguments should be ident")
                            }
                        }
                    }
                } else {
                    bail_span!(path, "Unknown pyimpl attribute")
                }
            }
            attr => bail_span!(attr, "Unknown pyimpl attribute"),
        }
    }

    Ok(ExtractedImplAttrs {
        with_impl: quote! {
            #(#withs)*
        },
        flags: quote! {
            #(#flags)*
        },
        with_slots: quote! {
            #(#with_slots)*
        },
    })
}

fn new_impl_item<Item>(index: usize, attr_name: String) -> Box<dyn ImplItem<Item>>
where
    Item: ItemLike + ToTokens + GetIdent,
{
    assert!(ALL_ALLOWED_NAMES.contains(&attr_name.as_str()));
    match attr_name.as_str() {
        attr_name @ "pymethod" | attr_name @ "pyclassmethod" => Box::new(MethodItem {
            inner: ContentItemInner {
                index,
                attr_name: attr_name.to_owned(),
            },
            method_type: attr_name.strip_prefix("py").unwrap().to_owned(),
        }),
        "pyproperty" => Box::new(PropertyItem {
            inner: ContentItemInner { index, attr_name },
        }),
        "pyslot" => Box::new(SlotItem {
            inner: ContentItemInner { index, attr_name },
        }),
        "pyattr" => Box::new(AttributeItem {
            inner: ContentItemInner { index, attr_name },
        }),
        "extend_class" => Box::new(ExtendClassItem {
            inner: ContentItemInner { index, attr_name },
        }),
        other => unreachable!("#[pyimpl] doesn't accept #[{}]", other),
    }
}

fn attrs_to_content_items<F, R>(
    attrs: &[Attribute],
    new_item: F,
) -> Result<(Vec<R>, Vec<Attribute>)>
where
    F: Fn(usize, String) -> R,
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
            return Err(syn::Error::new_spanned(
                attr,
                "#[py*] items must be placed under `cfgs`",
            ));
        }
        if !ALL_ALLOWED_NAMES.contains(&attr_name.as_str()) {
            continue;
        }

        result.push(new_item(i, attr_name));
    }
    Ok((result, cfgs))
}
