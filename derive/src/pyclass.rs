use super::Diagnostic;
use crate::util::{
    attribute_arg, def_to_name, meta_into_nesteds, optional_attribute_arg, path_eq, ItemIdent,
    ItemMeta, ItemType,
};
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{quote, quote_spanned, ToTokens};
use std::collections::HashMap;
use syn::{
    parse_quote, spanned::Spanned, Attribute, AttributeArgs, Ident, Index, Item, Lit, Meta,
    NestedMeta,
};

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

    fn extract_method(ident: &Ident, nesteds: Vec<NestedMeta>) -> Result<ClassItem, Diagnostic> {
        let item_meta =
            ItemMeta::from_nested_meta("pymethod", &ident, &nesteds, ItemMeta::ATTRIBUTE_NAMES)?;
        Ok(ClassItem::Method {
            item_ident: ident.clone(),
            py_name: item_meta.method_name()?,
        })
    }

    fn extract_classmethod(
        ident: &Ident,
        nesteds: Vec<NestedMeta>,
    ) -> Result<ClassItem, Diagnostic> {
        let item_meta = ItemMeta::from_nested_meta(
            "pyclassmethod",
            &ident,
            &nesteds,
            ItemMeta::ATTRIBUTE_NAMES,
        )?;
        Ok(ClassItem::ClassMethod {
            item_ident: ident.clone(),
            py_name: item_meta.method_name()?,
        })
    }

    fn extract_property(ident: &Ident, nesteds: Vec<NestedMeta>) -> Result<ClassItem, Diagnostic> {
        let item_meta =
            ItemMeta::from_nested_meta("pyproperty", &ident, &nesteds, ItemMeta::PROPERTY_NAMES)?;
        Ok(ClassItem::Property {
            py_name: item_meta.property_name()?,
            item_ident: ident.clone(),
            setter: item_meta.setter()?,
        })
    }

    fn extract_slot(ident: &Ident, meta: Meta) -> Result<ClassItem, Diagnostic> {
        let pyslot_err = "#[pyslot] must be of the form #[pyslot] or #[pyslot(slotname)]";
        let nesteds = meta_into_nesteds(meta).map_err(|meta| err_span!(meta, "{}", pyslot_err))?;
        if nesteds.len() > 1 {
            return Err(Diagnostic::spanned_error(&quote!(#(#nesteds)*), pyslot_err));
        }
        let slot_ident = if nesteds.is_empty() {
            let ident_str = ident.to_string();
            if let Some(stripped) = ident_str.strip_prefix("tp_") {
                proc_macro2::Ident::new(stripped, ident.span())
            } else {
                ident.clone()
            }
        } else {
            match nesteds.into_iter().next().unwrap() {
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
        for (i, meta) in item
            .attrs
            .iter()
            .filter_map(|attr| attr.parse_meta().ok())
            .enumerate()
        {
            let meta_span = meta.span();
            let name = match meta.path().get_ident() {
                Some(name) => name,
                None => continue,
            };
            assert!(item.typ == ItemType::Method);

            let into_nested = || {
                meta_into_nesteds(meta.clone()).map_err(|meta| {
                    err_span!(
                        meta,
                        "#[{name} = \"...\"] cannot be a name/value, you probably meant \
                         #[{name}(name = \"...\")]",
                        name = name.to_string(),
                    )
                })
            };
            let item = match name.to_string().as_str() {
                "pymethod" => Self::extract_method(item.ident, into_nested()?)?,
                "pyclassmethod" => Self::extract_classmethod(item.ident, into_nested()?)?,
                "pyproperty" => Self::extract_property(item.ident, into_nested()?)?,
                "pyslot" => Self::extract_slot(item.ident, meta)?,
                _ => {
                    continue;
                }
            };
            self.add_item(item, meta_span)?;
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
                quote! { ::rustpython_vm::__exports::smallbox! }
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
                            typ: ItemType::Method,
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
                            typ: ItemType::Method,
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
    tp_name: &str,
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

    let ret = quote! {
        impl ::rustpython_vm::pyobject::PyClassDef for #ident {
            const NAME: &'static str = #name;
            const TP_NAME: &'static str = #tp_name;
            const DOC: Option<&'static str> = #doc;
        }
    };
    Ok(ret)
}

pub fn impl_pyclass(attr: AttributeArgs, item: Item) -> Result<TokenStream2, Diagnostic> {
    let (item, ident, attrs) = match item {
        Item::Struct(struc) => (quote!(#struc), struc.ident, struc.attrs),
        Item::Enum(enu) => (quote!(#enu), enu.ident, enu.attrs),
        other => bail_span!(
            other,
            "#[pyclass] can only be on a struct or enum declaration"
        ),
    };

    let class_name = def_to_name("pyclass", &ident, &attr)?;
    let module_class_name =
        if let Some(module_name) = optional_attribute_arg("pystruct_sequence", "module", &attr)? {
            format!("{}.{}", module_name, class_name)
        } else {
            class_name.clone()
        };
    let class_def = generate_class_def(&ident, &class_name, &module_class_name, &attrs)?;

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
    let module_name = attribute_arg("pystruct_sequence", "module", &attr)?;
    let class_name = def_to_name("pystruct_sequence", &struc.ident, &attr)?;
    let module_class_name = format!("{}.{}", module_name, class_name);

    let class_def =
        generate_class_def(&struc.ident, &class_name, &module_class_name, &struc.attrs)?;
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
                let py_class = ctx.new_class(<Self as ::rustpython_vm::pyobject::PyClassDef>::NAME, ctx.tuple_type());
                Self::extend_class(ctx, &py_class);
                py_class
            }
        }
    };
    Ok(ret)
}
