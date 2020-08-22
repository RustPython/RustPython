use super::Diagnostic;
use crate::util::{
    path_eq, AttributeExt, ClassItemMeta, ContentItem, ContentItemInner, ItemIdent, ItemMeta,
    ItemMetaInner,
};
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{quote, quote_spanned, ToTokens};
use std::collections::HashMap;
use syn::{
    parse_quote, spanned::Spanned, Attribute, AttributeArgs, Ident, Index, Item, Lit, Meta,
    NestedMeta,
};
use syn_ext::types::*;

struct MethodItem {
    pub inner: ContentItemInner,
}
struct PropertyItem {
    pub inner: ContentItemInner,
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

#[derive(Default)]
struct Class {
    // Unlike pymodule, meta variant is not supported
    items: HashMap<String, ClassItem>,
}

#[derive(PartialEq, Eq, Hash)]
enum ClassItem {
    Method {
        item_ident: Ident,
        py_name: String,
    },
    ClassMethod {
        item_ident: Ident,
        py_name: String,
    },
    Property {
        item_ident: Ident,
        py_name: String,
        setter: bool,
    },
    Slot {
        slot_ident: Ident,
        item_ident: Ident,
    },
}

impl ClassItem {
    fn name(&self) -> String {
        use ClassItem::*;
        match self {
            Method { py_name, .. } => py_name.clone(),
            ClassMethod { py_name, .. } => py_name.clone(),
            Property {
                py_name, setter, ..
            } => {
                if *setter {
                    format!("{}.setter", py_name)
                } else {
                    py_name.clone()
                }
            }
            Slot { slot_ident, .. } => format!("#slot({})", slot_ident),
        }
    }
}

impl Class {
    fn add_item(&mut self, item: ClassItem, span: Span) -> Result<(), Diagnostic> {
        if let Some(existing) = self.items.insert(item.name(), item) {
            Err(Diagnostic::span_error(
                span,
                format!("Duplicate #[py*] attribute on pyimpl: {}", existing.name()),
            ))
        } else {
            Ok(())
        }
    }

    fn extract_method(
        item: &Ident,
        (meta, nested): (&Ident, PunctuatedNestedMeta),
    ) -> Result<ClassItem, Diagnostic> {
        let item_meta =
            MethodItemMeta::from_nested(item.clone(), meta.clone(), nested.into_iter())?;
        Ok(ClassItem::Method {
            item_ident: item.clone(),
            py_name: item_meta.method_name()?,
        })
    }

    fn extract_classmethod(
        item: &Ident,
        (meta, nested): (&Ident, PunctuatedNestedMeta),
    ) -> Result<ClassItem, Diagnostic> {
        let item_meta =
            MethodItemMeta::from_nested(item.clone(), meta.clone(), nested.into_iter())?;
        Ok(ClassItem::ClassMethod {
            item_ident: item.clone(),
            py_name: item_meta.method_name()?,
        })
    }

    fn extract_property(
        item: &Ident,
        (meta, nested): (&Ident, PunctuatedNestedMeta),
    ) -> Result<ClassItem, Diagnostic> {
        let item_meta =
            PropertyItemMeta::from_nested(item.clone(), meta.clone(), nested.into_iter())?;
        Ok(ClassItem::Property {
            item_ident: item.clone(),
            py_name: item_meta.property_name()?,
            setter: item_meta.setter()?,
        })
    }

    fn extract_slot(ident: &Ident, nested: PunctuatedNestedMeta) -> Result<ClassItem, Diagnostic> {
        let pyslot_err = "#[pyslot] must be of the form #[pyslot] or #[pyslot(slotname)]";
        if nested.len() > 1 {
            return Err(Diagnostic::spanned_error(&quote!(#nested), pyslot_err));
        }
        let slot_ident = if nested.is_empty() {
            let ident_str = ident.to_string();
            if let Some(stripped) = ident_str.strip_prefix("tp_") {
                proc_macro2::Ident::new(stripped, ident.span())
            } else {
                ident.clone()
            }
        } else {
            match nested.into_iter().next().unwrap() {
                NestedMeta::Meta(Meta::Path(path)) => path
                    .get_ident()
                    .cloned()
                    .ok_or_else(|| err_span!(path, "{}", pyslot_err))?,
                bad => bail_span!(bad, "{}", pyslot_err),
            }
        };
        Ok(ClassItem::Slot {
            slot_ident,
            item_ident: ident.clone(),
        })
    }

    fn extract_item_from_syn(&mut self, item: &mut ItemIdent) -> Result<(), Diagnostic> {
        let mut attr_idxs = Vec::new();
        for (i, attr) in item.attrs.iter_mut().enumerate() {
            let name = match attr.path.get_ident() {
                Some(name) => name,
                None => continue,
            };

            let item = match name.to_string().as_str() {
                "pymethod" => Self::extract_method(item.ident, attr.ident_and_promoted_nested()?)?,
                "pyclassmethod" => {
                    Self::extract_classmethod(item.ident, attr.ident_and_promoted_nested()?)?
                }
                "pyproperty" => {
                    Self::extract_property(item.ident, attr.ident_and_promoted_nested()?)?
                }
                "pyslot" => Self::extract_slot(item.ident, attr.promoted_nested()?)?,
                _ => {
                    continue;
                }
            };
            self.add_item(item, attr.span())?;
            attr_idxs.push(i);
        }
        let mut i = 0;
        let mut attr_idxs = &*attr_idxs;
        item.attrs.retain(|_| {
            let drop = attr_idxs.first().copied() == Some(i);
            if drop {
                attr_idxs = &attr_idxs[1..];
            }
            i += 1;
            !drop
        });
        for (i, idx) in attr_idxs.iter().enumerate() {
            item.attrs.remove(idx - i);
        }
        Ok(())
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
    fn method_name(&self) -> Result<String, Diagnostic> {
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
    fn property_name(&self) -> Result<String, Diagnostic> {
        let inner = self.inner();
        let magic = inner._bool("magic")?;
        let setter = inner._bool("setter")?;
        let name = inner._optional_str("name")?;

        Ok(if let Some(name) = name {
            name
        } else {
            let sig_name = inner.item_name();
            let name = if setter {
                if let Some(name) = sig_name.strip_prefix("set_") {
                    if name.is_empty() {
                        bail_span!(
                            &inner.meta_ident,
                            "A #[{}(setter)] fn with a set_* name must \
                             have something after \"set_\"",
                            inner.meta_name()
                        )
                    }
                    name.to_string()
                } else {
                    bail_span!(
                        &inner.meta_ident,
                        "A #[{}(setter)] fn must either have a `name` \
                         parameter or a fn name along the lines of \"set_*\"",
                        inner.meta_name()
                    )
                }
            } else {
                sig_name
            };
            if magic {
                format!("__{}__", name)
            } else {
                name
            }
        })
    }

    fn setter(&self) -> syn::Result<bool> {
        self.inner()._bool("setter")
    }
}

fn extract_impl_items(mut items: Vec<ItemIdent>) -> Result<TokenStream2, Diagnostic> {
    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    let mut class = Class::default();

    for item in items.iter_mut() {
        push_diag_result!(diagnostics, class.extract_item_from_syn(item),);
    }

    let mut properties: HashMap<&str, (Option<&Ident>, Option<&Ident>)> = HashMap::new();
    for item in class.items.values() {
        if let ClassItem::Property {
            ref item_ident,
            ref py_name,
            setter,
        } = item
        {
            let entry = properties.entry(py_name).or_default();
            let func = if *setter { &mut entry.1 } else { &mut entry.0 };
            if func.is_some() {
                bail_span!(
                    item_ident,
                    "Multiple property accessors with name {:?}",
                    py_name
                )
            }
            *func = Some(item_ident);
        }
    }
    let properties = properties
        .into_iter()
        .map(|(name, prop)| {
            let getter_func = match prop.0 {
                Some(func) => func,
                None => {
                    push_err_span!(
                        diagnostics,
                        prop.1.unwrap(),
                        "Property {:?} is missing a getter",
                        name
                    );
                    return TokenStream2::new();
                }
            };
            let (new, setter) = match prop.1 {
                Some(func) => (quote! { with_get_set }, quote! { , &Self::#func }),
                None => (quote! { with_get }, quote! { }),
            };
            let str_name = name.to_string();
            quote! {
                class.set_str_attr(
                    #name,
                    ::rustpython_vm::pyobject::PyObject::new(
                        ::rustpython_vm::obj::objgetset::PyGetSet::#new(#str_name.into(), &Self::#getter_func #setter),
                        ctx.getset_type(), None)
                );
            }
        })
        .collect::<Vec<_>>();
    let methods = class.items.values().filter_map(|item| match item {
        ClassItem::Method {
            item_ident,
            py_name,
        } => {
            let new_meth = quote_spanned!(item_ident.span()=> .new_method(Self::#item_ident));
            Some(quote! {
                class.set_str_attr(#py_name, ctx#new_meth);
            })
        }
        ClassItem::ClassMethod {
            item_ident,
            py_name,
        } => {
            let new_meth = quote_spanned!(item_ident.span()=> .new_classmethod(Self::#item_ident));
            Some(quote! {
                   class.set_str_attr(#py_name, ctx#new_meth);
            })
        }
        ClassItem::Slot {
            slot_ident,
            item_ident,
        } => {
            let transform = if vec!["new", "call"].contains(&slot_ident.to_string().as_str()) {
                quote! { ::rustpython_vm::function::IntoPyNativeFunc::into_func }
            } else {
                quote! { ::std::boxed::Box::new }
            };
            let into_func = quote_spanned! {item_ident.span()=>
                #transform(Self::#item_ident)
            };
            Some(quote! {
                (*class.slots.write()).#slot_ident = Some(#into_func);
            })
        }
        _ => None,
    });

    Diagnostic::from_vec(diagnostics)?;

    Ok(quote! {
        #(#methods)*
        #(#properties)*
    })
}

fn extract_impl_attrs(attr: AttributeArgs) -> Result<(TokenStream2, TokenStream2), Diagnostic> {
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

pub fn impl_pyimpl(attr: AttributeArgs, item: Item) -> Result<TokenStream2, Diagnostic> {
    match item {
        Item::Impl(mut imp) => {
            let items = imp
                .items
                .iter_mut()
                .filter_map(|item| match item {
                    syn::ImplItem::Method(syn::ImplItemMethod { attrs, sig, .. }) => {
                        Some(ItemIdent {
                            attrs,
                            ident: &sig.ident,
                        })
                    }
                    _ => None,
                })
                .collect();
            let extend_impl = extract_impl_items(items)?;
            let (with_impl, flags) = extract_impl_attrs(attr)?;
            let ty = &imp.self_ty;
            let ret = quote! {
                #imp
                impl ::rustpython_vm::pyobject::PyClassImpl for #ty {
                    const TP_FLAGS: ::rustpython_vm::slots::PyTpFlags = ::rustpython_vm::slots::PyTpFlags::from_bits_truncate(#flags);

                    fn impl_extend_class(
                        ctx: &::rustpython_vm::pyobject::PyContext,
                        class: &::rustpython_vm::obj::objtype::PyClassRef,
                    ) {
                        #extend_impl
                        #with_impl
                    }
                }
            };
            Ok(ret)
        }
        Item::Trait(mut trai) => {
            let items = trai
                .items
                .iter_mut()
                .filter_map(|item| match item {
                    syn::TraitItem::Method(syn::TraitItemMethod { attrs, sig, .. }) => {
                        Some(ItemIdent {
                            attrs,
                            ident: &sig.ident,
                        })
                    }
                    _ => None,
                })
                .collect();
            let extend_impl = extract_impl_items(items)?;
            let item = parse_quote! {
                fn __extend_py_class(
                    ctx: &::rustpython_vm::pyobject::PyContext,
                    class: &::rustpython_vm::obj::objtype::PyClassRef,
                ) {
                    #extend_impl
                }
            };
            trai.items.push(item);
            Ok(trai.into_token_stream())
        }
        item => Ok(quote!(#item)),
    }
}

fn generate_class_def(
    ident: &Ident,
    name: &str,
    module_name: Option<&str>,
    attrs: &[Attribute],
) -> Result<TokenStream2, Diagnostic> {
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

    let ret = quote! {
        impl ::rustpython_vm::pyobject::PyClassDef for #ident {
            const NAME: &'static str = #name;
            const MODULE_NAME: Option<&'static str> = #module_name;
            const TP_NAME: &'static str = #module_class_name;
            const DOC: Option<&'static str> = #doc;
        }
    };
    Ok(ret)
}

pub fn impl_pyclass(attr: AttributeArgs, item: Item) -> Result<TokenStream2, Diagnostic> {
    let (ident, attrs) = match &item {
        Item::Struct(syn::ItemStruct { ident, attrs, .. }) => (ident, attrs),
        Item::Enum(syn::ItemEnum { ident, attrs, .. }) => (ident, attrs),
        other => bail_span!(
            other,
            "#[pyclass] can only be on a struct or enum declaration"
        ),
    };

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

pub fn impl_pystruct_sequence(attr: AttributeArgs, item: Item) -> Result<TokenStream2, Diagnostic> {
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
