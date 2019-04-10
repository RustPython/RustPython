use super::rustpython_path_attr;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use std::collections::HashMap;
use syn::{Attribute, AttributeArgs, Ident, ImplItem, Item, Lit, Meta, MethodSig, NestedMeta};

#[derive(PartialEq, Clone, Copy)]
enum ClassItemKind {
    Method,
    PropertyGetter,
    PropertySetter,
}

struct ClassItem {
    item_name: Ident,
    py_name: String,
    kind: ClassItemKind,
}

fn meta_to_vec(meta: Meta) -> Result<Vec<NestedMeta>, Meta> {
    match meta {
        Meta::Word(_) => Ok(Vec::new()),
        Meta::List(list) => Ok(list.nested.into_iter().collect()),
        Meta::NameValue(_) => Err(meta),
    }
}

impl ClassItem {
    fn extract_from_syn(attrs: &mut Vec<Attribute>, sig: &MethodSig) -> Option<ClassItem> {
        let mut item = None;
        let mut attr_idx = None;
        for (i, meta) in attrs
            .iter()
            .filter_map(|attr| attr.parse_meta().ok())
            .enumerate()
        {
            let name = meta.name();
            if name == "pymethod" {
                if item.is_some() {
                    panic!("You can only have one #[py*] attribute on an impl item")
                }
                let nesteds = meta_to_vec(meta)
                    .expect("#[pymethod = ...] must be a list, e.g. #[pymethod(...)]");
                let mut py_name = None;
                for meta in nesteds {
                    let meta = match meta {
                        NestedMeta::Meta(meta) => meta,
                        NestedMeta::Literal(_) => panic!("Expected a meta, found a literal"),
                    };
                    match meta {
                        Meta::NameValue(name_value) => {
                            if name_value.ident == "name" {
                                if let Lit::Str(s) = &name_value.lit {
                                    py_name = Some(s.value());
                                } else {
                                    panic!("#[pymethod(name = ...)] must be a string");
                                }
                            }
                        }
                        _ => {}
                    }
                }
                item = Some(ClassItem {
                    item_name: sig.ident.clone(),
                    py_name: py_name.unwrap_or_else(|| sig.ident.to_string()),
                    kind: ClassItemKind::Method,
                });
                attr_idx = Some(i);
            } else if name == "pyproperty" {
                if item.is_some() {
                    panic!("You can only have one #[py*] attribute on an impl item")
                }
                let nesteds = meta_to_vec(meta)
                    .expect("#[pyproperty = ...] must be a list, e.g. #[pyproperty(...)]");
                let mut setter = false;
                let mut py_name = None;
                for meta in nesteds {
                    let meta = match meta {
                        NestedMeta::Meta(meta) => meta,
                        NestedMeta::Literal(_) => panic!("Expected a meta, found a literal"),
                    };
                    match meta {
                        Meta::NameValue(name_value) => {
                            if name_value.ident == "name" {
                                if let Lit::Str(s) = &name_value.lit {
                                    py_name = Some(s.value());
                                } else {
                                    panic!("#[pyproperty(name = ...)] must be a string");
                                }
                            }
                        }
                        Meta::Word(ident) => {
                            if ident == "setter" {
                                setter = true;
                            }
                        }
                        _ => {}
                    }
                }
                let kind = if setter {
                    ClassItemKind::PropertySetter
                } else {
                    ClassItemKind::PropertyGetter
                };
                let py_name = py_name.unwrap_or_else(|| {
                    let item_name = sig.ident.to_string();
                    if item_name.starts_with("set_") {
                        let name = &item_name["set_".len()..];
                        if name.is_empty() {
                            panic!(
                                "A #[pyproperty(setter)] fn with a set_* name have something \
                                 after \"set_\""
                            )
                        } else {
                            name.to_string()
                        }
                    } else {
                        panic!(
                            "A #[pyproperty(setter)] fn must either have a `name` parameter or a \
                             fn name along the lines of \"set_*\""
                        )
                    }
                });
                item = Some(ClassItem {
                    py_name,
                    item_name: sig.ident.clone(),
                    kind,
                });
                attr_idx = Some(i);
            }
        }
        if let Some(attr_idx) = attr_idx {
            attrs.remove(attr_idx);
        }
        item
    }
}

pub fn impl_pyimpl(attr: AttributeArgs, item: Item) -> TokenStream2 {
    let mut imp = if let Item::Impl(imp) = item {
        imp
    } else {
        return quote!(#item);
    };

    let rp_path = rustpython_path_attr(&attr);

    let items = imp
        .items
        .iter_mut()
        .filter_map(|item| {
            if let ImplItem::Method(meth) = item {
                ClassItem::extract_from_syn(&mut meth.attrs, &meth.sig)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    let ty = &imp.self_ty;
    let mut properties: HashMap<&str, (Option<&Ident>, Option<&Ident>)> = HashMap::new();
    for item in items.iter() {
        match item.kind {
            ClassItemKind::PropertyGetter => {
                let (ref mut getter, _) = properties.entry(&item.py_name).or_default();
                if getter.is_some() {
                    panic!("Multiple property getters with name {:?}", &item.py_name)
                }
                *getter = Some(&item.item_name);
            }
            ClassItemKind::PropertySetter => {
                let (_, ref mut setter) = properties.entry(&item.py_name).or_default();
                if setter.is_some() {
                    panic!("Multiple property getters with name {:?}", &item.py_name)
                }
                *setter = Some(&item.item_name);
            }
            ClassItemKind::Method => {}
        }
    }
    let methods = items.iter().filter_map(|item| {
        if let ClassItemKind::Method = item.kind {
            let ClassItem {
                py_name, item_name, ..
            } = item;
            Some(quote! {
                class.set_str_attr(#py_name, ctx.new_rustfunc(Self::#item_name));
            })
        } else {
            None
        }
    });
    let properties = properties.iter().map(|(name, prop)| {
        let getter = match prop.0 {
            Some(getter) => getter,
            None => panic!("Property {:?} is missing a getter", name),
        };
        let add_setter = prop.1.map(|setter| quote!(.add_setter(Self::#setter)));
        quote! {
            class.set_str_attr(
                #name,
                #rp_path::obj::objproperty::PropertyBuilder
                    .add_getter(Self::#getter)
                    #add_setter
                    .create(),
            );
        }
    });

    quote! {
        #imp
        impl #rp_path::pyobject::PyClassImpl for #ty {
            fn impl_extend_class(
                ctx: &#rp_path::pyobject::PyContext,
                class: &#rp_path::obj::objtype::PyClassRef,
            ) {
                #(#methods)*
                #(#properties)*
            }
        }
    }
}

pub fn impl_pyclass(attr: AttributeArgs, item: Item) -> TokenStream2 {
    let (item, ident, attrs) = match item {
        Item::Struct(struc) => (quote!(#struc), struc.ident, struc.attrs),
        Item::Enum(enu) => (quote!(#enu), enu.ident, enu.attrs),
        _ => panic!("#[pyclass] can only be on a struct or enum declaration"),
    };

    let rp_path = rustpython_path_attr(&attr);

    let mut class_name = None;
    for attr in attr {
        if let NestedMeta::Meta(meta) = attr {
            if let Meta::NameValue(name_value) = meta {
                if name_value.ident == "name" {
                    if let Lit::Str(s) = name_value.lit {
                        class_name = Some(s.value());
                    } else {
                        panic!("#[pyclass(name = ...)] must be a string");
                    }
                }
            }
        }
    }
    let class_name = class_name.unwrap_or_else(|| ident.to_string());

    let mut doc: Option<Vec<String>> = None;
    for attr in attrs.iter() {
        if attr.path.is_ident("doc") {
            let meta = attr.parse_meta().expect("expected doc attr to be a meta");
            if let Meta::NameValue(name_value) = meta {
                if let Lit::Str(s) = name_value.lit {
                    let val = s.value().trim().to_string();
                    match doc {
                        Some(ref mut doc) => doc.push(val),
                        None => doc = Some(vec![val]),
                    }
                } else {
                    panic!("expected #[doc = ...] to be a string")
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

    quote! {
        #item
        impl #rp_path::pyobject::PyClassDef for #ident {
            const NAME: &'static str = #class_name;
            const DOC: Option<&'static str> = #doc;
        }
    }
}
