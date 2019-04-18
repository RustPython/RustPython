use super::Diagnostic;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use std::collections::HashMap;
use syn::{Attribute, AttributeArgs, Ident, ImplItem, Item, Lit, Meta, MethodSig, NestedMeta};

enum ClassItem {
    Method {
        item_ident: Ident,
        py_name: String,
    },
    Property {
        item_ident: Ident,
        py_name: String,
        setter: bool,
    },
}

fn meta_to_vec(meta: Meta) -> Result<Vec<NestedMeta>, Meta> {
    match meta {
        Meta::Word(_) => Ok(Vec::new()),
        Meta::List(list) => Ok(list.nested.into_iter().collect()),
        Meta::NameValue(_) => Err(meta),
    }
}

impl ClassItem {
    fn extract_from_syn(
        attrs: &mut Vec<Attribute>,
        sig: &MethodSig,
    ) -> Result<Option<ClassItem>, Diagnostic> {
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
                    bail_span!(
                        sig.ident,
                        "You can only have one #[py*] attribute on an impl item"
                    )
                }
                let nesteds = meta_to_vec(meta).map_err(|meta| {
                    err_span!(
                        meta,
                        "#[pyproperty = \"...\"] cannot be a name/value, you probably meant \
                         #[pyproperty(name = \"...\")]",
                    )
                })?;
                let mut py_name = None;
                for meta in nesteds {
                    let meta = match meta {
                        NestedMeta::Meta(meta) => meta,
                        NestedMeta::Literal(_) => continue,
                    };
                    match meta {
                        Meta::NameValue(name_value) => {
                            if name_value.ident == "name" {
                                if let Lit::Str(s) = &name_value.lit {
                                    py_name = Some(s.value());
                                } else {
                                    bail_span!(
                                        &sig.ident,
                                        "#[pymethod(name = ...)] must be a string"
                                    );
                                }
                            }
                        }
                        _ => {}
                    }
                }
                item = Some(ClassItem::Method {
                    item_ident: sig.ident.clone(),
                    py_name: py_name.unwrap_or_else(|| sig.ident.to_string()),
                });
                attr_idx = Some(i);
            } else if name == "pyproperty" {
                if item.is_some() {
                    bail_span!(
                        sig.ident,
                        "You can only have one #[py*] attribute on an impl item"
                    )
                }
                let nesteds = meta_to_vec(meta).map_err(|meta| {
                    err_span!(
                        meta,
                        "#[pyproperty = \"...\"] cannot be a name/value, you probably meant \
                         #[pyproperty(name = \"...\")]"
                    )
                })?;
                let mut setter = false;
                let mut py_name = None;
                for meta in nesteds {
                    let meta = match meta {
                        NestedMeta::Meta(meta) => meta,
                        NestedMeta::Literal(_) => continue,
                    };
                    match meta {
                        Meta::NameValue(name_value) => {
                            if name_value.ident == "name" {
                                if let Lit::Str(s) = &name_value.lit {
                                    py_name = Some(s.value());
                                } else {
                                    bail_span!(
                                        &sig.ident,
                                        "#[pyproperty(name = ...)] must be a string"
                                    );
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
                let py_name = match py_name {
                    Some(py_name) => py_name,
                    None => {
                        let item_ident = sig.ident.to_string();
                        if setter {
                            if item_ident.starts_with("set_") {
                                let name = &item_ident["set_".len()..];
                                if name.is_empty() {
                                    bail_span!(
                                        &sig.ident,
                                        "A #[pyproperty(setter)] fn with a set_* name must \
                                         have something after \"set_\""
                                    )
                                } else {
                                    name.to_string()
                                }
                            } else {
                                bail_span!(
                                    &sig.ident,
                                    "A #[pyproperty(setter)] fn must either have a `name` \
                                     parameter or a fn name along the lines of \"set_*\""
                                )
                            }
                        } else {
                            item_ident
                        }
                    }
                };
                item = Some(ClassItem::Property {
                    py_name,
                    item_ident: sig.ident.clone(),
                    setter,
                });
                attr_idx = Some(i);
            }
        }
        if let Some(attr_idx) = attr_idx {
            attrs.remove(attr_idx);
        }
        Ok(item)
    }
}

pub fn impl_pyimpl(_attr: AttributeArgs, item: Item) -> Result<TokenStream2, Diagnostic> {
    let mut imp = if let Item::Impl(imp) = item {
        imp
    } else {
        return Ok(quote!(#item));
    };

    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    let items = imp
        .items
        .iter_mut()
        .filter_map(|item| {
            if let ImplItem::Method(meth) = item {
                ClassItem::extract_from_syn(&mut meth.attrs, &meth.sig)
                    .map_err(|err| diagnostics.push(err))
                    .unwrap_or_default()
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    let ty = &imp.self_ty;
    let mut properties: HashMap<&str, (Option<&Ident>, Option<&Ident>)> = HashMap::new();
    for item in items.iter() {
        match item {
            ClassItem::Property {
                ref item_ident,
                ref py_name,
                setter,
            } => {
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
            _ => {}
        }
    }
    let methods = items.iter().filter_map(|item| {
        if let ClassItem::Method {
            item_ident,
            py_name,
        } = item
        {
            Some(quote! {
                class.set_str_attr(#py_name, ctx.new_rustfunc(Self::#item_ident));
            })
        } else {
            None
        }
    });
    let properties = properties
        .iter()
        .map(|(name, prop)| {
            let getter = match prop.0 {
                Some(getter) => getter,
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
            let add_setter = prop.1.map(|setter| quote!(.add_setter(Self::#setter)));
            quote! {
                class.set_str_attr(
                    #name,
                    ::rustpython_vm::obj::objproperty::PropertyBuilder::new(ctx)
                        .add_getter(Self::#getter)
                        #add_setter
                        .create(),
                );
            }
        })
        .collect::<Vec<_>>();

    Diagnostic::from_vec(diagnostics)?;

    let ret = quote! {
        #imp
        impl ::rustpython_vm::pyobject::PyClassImpl for #ty {
            fn impl_extend_class(
                ctx: &::rustpython_vm::pyobject::PyContext,
                class: &::rustpython_vm::obj::objtype::PyClassRef,
            ) {
                #(#methods)*
                #(#properties)*
            }
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

    let mut class_name = None;
    for attr in attr {
        if let NestedMeta::Meta(meta) = attr {
            if let Meta::NameValue(name_value) = meta {
                if name_value.ident == "name" {
                    if let Lit::Str(s) = name_value.lit {
                        class_name = Some(s.value());
                    } else {
                        bail_span!(name_value.lit, "#[pyclass(name = ...)] must be a string");
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
        #item
        impl ::rustpython_vm::pyobject::PyClassDef for #ident {
            const NAME: &'static str = #class_name;
            const DOC: Option<&'static str> = #doc;
        }
    };
    Ok(ret)
}
