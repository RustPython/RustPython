use super::Diagnostic;
use crate::util::{
    path_eq, pyclass_ident_and_attrs, AttributeExt, ClassItemMeta, ContentItem, ContentItemInner,
    ErrorVec, ItemMeta, ItemMetaInner, ItemNursery, ALL_ALLOWED_NAMES,
};
use proc_macro2::TokenStream;
use quote::{quote, quote_spanned, ToTokens};
use std::collections::HashMap;
use syn::{
    parse_quote, spanned::Spanned, Attribute, AttributeArgs, Ident, Index, Item, Lit, Meta,
    NestedMeta, Result,
};
use syn_ext::ext::*;

#[derive(Default)]
struct ImplContext {
    impl_extend_items: ItemNursery,
    getset_items: GetSetNursery,
    errors: Vec<syn::Error>,
}

pub(crate) fn impl_pyimpl(
    attr: AttributeArgs,
    item: Item,
) -> std::result::Result<TokenStream, Diagnostic> {
    let mut context = ImplContext::default();
    let mut tokens = match item {
        Item::Impl(mut imp) => {
            for item in imp.items.iter_mut() {
                let r = item.try_split_attr_mut(|attrs, item| {
                    let (pyitems, cfgs) =
                        attrs_to_content_items(&attrs, new_impl_item::<syn::ImplItem>)?;
                    for pyitem in pyitems.iter().rev() {
                        let r = pyitem.gen_impl_item(ImplItemArgs {
                            item,
                            attrs,
                            context: &mut context,
                            cfgs: cfgs.as_slice(),
                        });
                        context.errors.ok_or_push(r);
                    }
                    Ok(())
                });
                context.errors.ok_or_push(r);
            }
            context.errors.ok_or_push(context.getset_items.validate());

            let (with_impl, flags) = extract_impl_attrs(attr)?;
            let ty = &imp.self_ty;
            let getset_impl = &context.getset_items;
            let extend_impl = &context.impl_extend_items;
            quote! {
                #imp
                impl ::rustpython_vm::pyobject::PyClassImpl for #ty {
                    const TP_FLAGS: ::rustpython_vm::slots::PyTpFlags = ::rustpython_vm::slots::PyTpFlags::from_bits_truncate(#flags);

                    fn impl_extend_class(
                        ctx: &::rustpython_vm::pyobject::PyContext,
                        class: &::rustpython_vm::obj::objtype::PyClassRef,
                    ) {
                        #getset_impl
                        #extend_impl
                        #with_impl
                    }
                }
            }
        }
        Item::Trait(mut trai) => {
            let mut context = ImplContext::default();
            for item in trai.items.iter_mut() {
                let r = item.try_split_attr_mut(|attrs, item| {
                    let (pyitems, cfgs) =
                        attrs_to_content_items(&attrs, new_impl_item::<syn::TraitItem>)?;
                    for pyitem in pyitems.iter().rev() {
                        let r = pyitem.gen_impl_item(ImplItemArgs {
                            item,
                            attrs,
                            context: &mut context,
                            cfgs: cfgs.as_slice(),
                        });
                        context.errors.ok_or_push(r);
                    }
                    Ok(())
                });
                context.errors.ok_or_push(r);
            }
            context.errors.ok_or_push(context.getset_items.validate());

            let getset_impl = &context.getset_items;
            let extend_impl = &context.impl_extend_items;
            trai.items.push(parse_quote! {
                fn __extend_py_class(
                    ctx: &::rustpython_vm::pyobject::PyContext,
                    class: &::rustpython_vm::obj::objtype::PyClassRef,
                ) {
                    #getset_impl
                    #extend_impl
                }
            });
            trai.into_token_stream()
        }
        item => quote!(#item),
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
    attrs: &[Attribute],
) -> std::result::Result<TokenStream, Diagnostic> {
    let mut doc: Option<Vec<String>> = None;
    for attr in attrs.iter() {
        if attr.path.is_ident("doc") {
            let meta = attr.parse_meta().expect("expected doc attr to be a meta");
            if let Meta::NameValue(name_value) = meta {
                if let Lit::Str(s) = name_value.lit {
                    let val = s.value().trim().to_owned();
                    match doc {
                        Some(ref mut doc) => doc.push(val),
                        None => doc = Some(vec![val]),
                    }
                }
            }
        }
    }
    let doc = match doc {
        Some(doc) => {
            let doc = doc.join("\n");
            quote!(Some(#doc))
        }
        None => quote!(None),
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

    let tokens = quote! {
        impl ::rustpython_vm::pyobject::PyClassDef for #ident {
            const NAME: &'static str = #name;
            const MODULE_NAME: Option<&'static str> = #module_name;
            const TP_NAME: &'static str = #module_class_name;
            const DOC: Option<&'static str> = #doc;
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
    let class_def = generate_class_def(&ident, &class_name, module_name.as_deref(), &attrs)?;

    let ret = quote! {
        #item
        #class_def
    };
    Ok(ret)
}

pub(crate) fn impl_pystruct_sequence(
    attr: AttributeArgs,
    item: Item,
) -> std::result::Result<TokenStream, Diagnostic> {
    let struc = if let Item::Struct(struc) = item {
        struc
    } else {
        bail_span!(
            item,
            "#[pystruct_sequence] can only be on a struct declaration"
        )
    };
    let fake_ident = Ident::new("pystruct_sequence", struc.span());
    let class_meta = ClassItemMeta::from_nested(struc.ident.clone(), fake_ident, attr.into_iter())?;
    let class_name = class_meta.class_name()?;
    let module_name = class_meta.mandatory_module()?;

    let class_def =
        generate_class_def(&struc.ident, &class_name, Some(&module_name), &struc.attrs)?;
    let mut properties = Vec::new();
    let mut field_names = Vec::new();
    for (i, field) in struc.fields.iter().enumerate() {
        let idx = Index::from(i);
        if let Some(ref field_name) = field.ident {
            let field_name_str = field_name.to_string();
            // TODO add doc to the generated property
            let property = quote! {
                class.set_str_attr(
                    #field_name_str,
                    ctx.new_readonly_getset(
                        #field_name_str,
                        |zelf: &::rustpython_vm::obj::objtuple::PyTuple,
                         _vm: &::rustpython_vm::VirtualMachine| {
                            zelf.fast_getitem(#idx)
                        }
                   ),
                );
            };
            properties.push(property);
            field_names.push(quote!(#field_name));
        } else {
            field_names.push(quote!(#idx));
        }
    }

    let ty = &struc.ident;
    let ret = quote! {
        #struc
        #class_def

        impl #ty {
            pub fn into_struct_sequence(&self,
                vm: &::rustpython_vm::VirtualMachine,
                cls: ::rustpython_vm::obj::objtype::PyClassRef,
            ) -> ::rustpython_vm::pyobject::PyResult<::rustpython_vm::obj::objtuple::PyTupleRef> {
                let tuple = ::rustpython_vm::obj::objtuple::PyTuple::from(
                    vec![#(::rustpython_vm::pyobject::IntoPyObject::into_pyobject(
                        ::std::clone::Clone::clone(&self.#field_names),
                        vm,
                    )),*],
                );
                ::rustpython_vm::pyobject::PyValue::into_ref_with_type(tuple, vm, cls)
            }
        }

        impl ::rustpython_vm::pyobject::PyStructSequenceImpl for #ty {
            const FIELD_NAMES: &'static [&'static str] = &[#(stringify!(#field_names)),*];
        }

        impl ::rustpython_vm::pyobject::PyClassImpl for #ty {
            #[cfg(debug_assertions)]
            const TP_FLAGS: ::rustpython_vm::slots::PyTpFlags = ::rustpython_vm::slots::PyTpFlags::_CREATED_WITH_FLAGS;

            fn impl_extend_class(
                ctx: &::rustpython_vm::pyobject::PyContext,
                class: &::rustpython_vm::obj::objtype::PyClassRef,
            ) {
                use ::rustpython_vm::pyobject::PyStructSequenceImpl;

                #(#properties)*
                class.set_str_attr("__repr__", ctx.new_method(Self::repr));
            }

            fn make_class(
                ctx: &::rustpython_vm::pyobject::PyContext,
            ) -> ::rustpython_vm::obj::objtype::PyClassRef {
                let py_class = Self::create_bare_type(&ctx.type_type(), &ctx.tuple_type());
                Self::extend_class(ctx, &py_class);
                py_class
            }
        }
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
        let item_meta = MethodItemMeta::from_nested(
            ident.clone(),
            item_attr.get_ident().unwrap().clone(),
            item_attr.promoted_nested()?.into_iter(),
        )?;

        let py_name = item_meta.method_name()?;
        let new_func = Ident::new(&format!("new_{}", &self.method_type), args.item.span());
        let tokens = {
            let new_func = quote_spanned!(
                ident.span() => .#new_func(Self::#ident)
            );
            quote! {
                class.set_str_attr(#py_name, ctx#new_func);
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
    fn gen_impl_item(&self, args: ImplItemArgs<'_, Item>) -> Result<()>
    where
        Item: ItemLike + ToTokens + GetIdent,
    {
        let ident = if args.item.is_function_or_method() {
            Ok(args.item.get_ident().unwrap())
        } else {
            Err(self.new_syn_error(args.item.span(), "can only be on a method"))
        }?;

        let item_attr = args.attrs.remove(self.index());
        let item_meta = PropertyItemMeta::from_nested(
            ident.clone(),
            item_attr.get_ident().unwrap().clone(),
            item_attr.promoted_nested()?.into_iter(),
        )?;

        let py_name = item_meta.property_name()?;
        let setter = item_meta.setter()?;
        args.context
            .getset_items
            .add_item(py_name, args.cfgs.to_vec(), setter, ident.clone())?;
        Ok(())
    }
}

impl<Item> ImplItem<Item> for SlotItem
where
    Item: ItemLike + ToTokens + GetIdent,
{
    fn gen_impl_item(&self, args: ImplItemArgs<'_, Item>) -> Result<()>
    where
        Item: ItemLike + ToTokens + GetIdent,
    {
        let ident = if args.item.is_function_or_method() {
            Ok(args.item.get_ident().unwrap())
        } else {
            Err(self.new_syn_error(args.item.span(), "can only be on a method"))
        }?;

        let item_attr = args.attrs.remove(self.index());
        let item_meta = SlotItemMeta::from_nested(
            ident.clone(),
            item_attr.get_ident().unwrap().clone(),
            item_attr.promoted_nested()?.into_iter(),
        )?;

        let slot_ident = item_meta.slot_name()?;
        let slot_name = slot_ident.to_string();
        let tokens = {
            let transform = if vec!["new", "call"].contains(&slot_name.as_str()) {
                quote! { ::rustpython_vm::function::IntoPyNativeFunc::into_func }
            } else {
                quote! { ::std::boxed::Box::new }
            };
            let into_func = quote_spanned! {ident.span() =>
                #transform(Self::#ident)
            };
            quote! {
                (*class.slots.write()).#slot_ident = Some(#into_func);
            }
        };

        args.context.impl_extend_items.add_item(
            format!("(slot {})", slot_name),
            args.cfgs.to_vec(),
            tokens,
        )?;

        Ok(())
    }
}

#[derive(Default)]
#[allow(clippy::type_complexity)]
struct GetSetNursery {
    map: HashMap<(String, Vec<Attribute>), (Option<Ident>, Option<Ident>)>,
    validated: bool,
}

impl GetSetNursery {
    fn add_item(
        &mut self,
        name: String,
        cfgs: Vec<Attribute>,
        setter: bool,
        item_ident: Ident,
    ) -> Result<()> {
        assert!(!self.validated, "new item is not allowed after validation");
        if setter && !cfgs.is_empty() {
            return Err(syn::Error::new_spanned(
                item_ident,
                "Property setter does not allow #[cfg]",
            ));
        }
        let entry = self.map.entry((name.clone(), cfgs)).or_default();
        let func = if setter { &mut entry.1 } else { &mut entry.0 };
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
        for ((name, _cfgs), (getter, setter)) in self.map.iter() {
            if getter.is_none() {
                errors.push(syn::Error::new_spanned(
                    setter.as_ref().unwrap(),
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
        tokens.extend(self.map.iter().map(|((name, cfgs), (getter, setter))| {
            let (constructor, setter) = match setter {
                Some(setter) => (quote! { with_get_set }, quote_spanned! { setter.span() => , &Self::#setter }),
                None => (quote! { with_get }, quote! { }),
            };
            quote! {
                #( #cfgs )*
                class.set_str_attr(
                    #name,
                    ::rustpython_vm::pyobject::PyObject::new(
                        ::rustpython_vm::obj::objgetset::PyGetSet::#constructor(#name.into(), &Self::#getter #setter),
                        ctx.getset_type(), None)
                );
            }
        }));
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
    const ALLOWED_NAMES: &'static [&'static str] = &["name", "magic", "setter"];

    fn from_inner(inner: ItemMetaInner) -> Self {
        Self(inner)
    }
    fn inner(&self) -> &ItemMetaInner {
        &self.0
    }
}

impl PropertyItemMeta {
    fn property_name(&self) -> Result<String> {
        let inner = self.inner();
        let magic = inner._bool("magic")?;
        let setter = inner._bool("setter")?;
        let name = inner._optional_str("name")?;
        let py_name = if let Some(name) = name {
            name
        } else {
            let sig_name = inner.item_name();
            let name = if setter {
                if let Some(name) = sig_name.strip_prefix("set_") {
                    if name.is_empty() {
                        return Err(syn::Error::new_spanned(
                            &inner.meta_ident,
                            format!(
                                "A #[{}(setter)] fn with a set_* name must \
                                 have something after \"set_\"",
                                inner.meta_name()
                            ),
                        ));
                    }
                    name.to_string()
                } else {
                    return Err(syn::Error::new_spanned(
                        &inner.meta_ident,
                        format!(
                            "A #[{}(setter)] fn must either have a `name` \
                             parameter or a fn name along the lines of \"set_*\"",
                            inner.meta_name()
                        ),
                    ));
                }
            } else {
                sig_name
            };
            if magic {
                format!("__{}__", name)
            } else {
                name
            }
        };
        Ok(py_name)
    }

    fn setter(&self) -> Result<bool> {
        self.inner()._bool("setter")
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

fn extract_impl_attrs(
    attr: AttributeArgs,
) -> std::result::Result<(TokenStream, TokenStream), Diagnostic> {
    let mut withs = Vec::new();
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
                        match meta {
                            NestedMeta::Meta(Meta::Path(path)) => {
                                withs.push(quote! {
                                    <Self as #path>::__extend_py_class(ctx, class);
                                });
                            }
                            meta => {
                                bail_span!(meta, "#[pyimpl(with(...))] arguments should be paths")
                            }
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

    Ok((
        quote! {
            #(#withs)*
        },
        quote! {
            #(#flags)*
        },
    ))
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
