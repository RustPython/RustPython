use super::Diagnostic;
use crate::util::{def_to_name, meta_into_nesteds, ItemIdent, ItemMeta, ItemType};
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{quote, quote_spanned, ToTokens};
use std::collections::HashMap;
use syn::{parse_quote, spanned::Spanned, AttributeArgs, Ident, Item, Meta, NestedMeta};

#[derive(Default)]
struct Module {
    items: HashMap<(String, Vec<Meta>), ModuleItem>,
}

#[derive(PartialEq, Eq, Hash)]
enum ModuleItem {
    Function { item_ident: Ident, py_name: String },
    EvaluatedAttr { item_ident: Ident, py_name: String },
    ConstAttr { item_ident: Ident, py_name: String },
    Class { item_ident: Ident, py_name: String },
}

impl ModuleItem {
    fn name(&self) -> String {
        use ModuleItem::*;
        match self {
            Function { py_name, .. } => py_name.clone(),
            EvaluatedAttr { py_name, .. } => py_name.clone(),
            ConstAttr { py_name, .. } => py_name.clone(),
            Class { py_name, .. } => py_name.clone(),
        }
    }
}

impl Module {
    fn add_item(
        &mut self,
        item: ModuleItem,
        cfgs: Vec<Meta>,
        span: Span,
    ) -> Result<(), Diagnostic> {
        if let Some(existing) = self.items.insert((item.name(), cfgs), item) {
            Err(Diagnostic::span_error(
                span,
                format!(
                    "Duplicate #[py*] attribute on pymodule: {}",
                    existing.name()
                ),
            ))
        } else {
            Ok(())
        }
    }

    fn extract_function(ident: &Ident, nesteds: Vec<NestedMeta>) -> Result<ModuleItem, Diagnostic> {
        let item_meta =
            ItemMeta::from_nested_meta("pyfunction", &ident, &nesteds, ItemMeta::SIMPLE_NAMES)?;
        Ok(ModuleItem::Function {
            item_ident: ident.clone(),
            py_name: item_meta.simple_name()?,
        })
    }

    fn extract_class(ident: &Ident, nesteds: Vec<NestedMeta>) -> Result<ModuleItem, Diagnostic> {
        let item_meta =
            ItemMeta::from_nested_meta("pyclass", &ident, &nesteds, ItemMeta::SIMPLE_NAMES)?;
        Ok(ModuleItem::Class {
            item_ident: ident.clone(),
            py_name: item_meta.simple_name()?,
        })
    }

    fn extract_struct_sequence(
        ident: &Ident,
        nesteds: Vec<NestedMeta>,
    ) -> Result<ModuleItem, Diagnostic> {
        let item_meta = ItemMeta::from_nested_meta(
            "pystruct_sequence",
            &ident,
            &nesteds,
            ItemMeta::STRUCT_SEQUENCE_NAMES,
        )?;
        Ok(ModuleItem::Class {
            item_ident: ident.clone(),
            py_name: item_meta.simple_name()?,
        })
    }

    fn extract_evaluated_attr(
        ident: &Ident,
        nesteds: Vec<NestedMeta>,
    ) -> Result<ModuleItem, Diagnostic> {
        let item_meta =
            ItemMeta::from_nested_meta("pyattr", &ident, &nesteds, ItemMeta::SIMPLE_NAMES)?;
        Ok(ModuleItem::EvaluatedAttr {
            item_ident: ident.clone(),
            py_name: item_meta.simple_name()?,
        })
    }

    fn extract_const_attr(
        ident: &Ident,
        nesteds: Vec<NestedMeta>,
    ) -> Result<ModuleItem, Diagnostic> {
        let item_meta =
            ItemMeta::from_nested_meta("pyattr", &ident, &nesteds, ItemMeta::SIMPLE_NAMES)?;
        Ok(ModuleItem::ConstAttr {
            item_ident: ident.clone(),
            py_name: item_meta.simple_name()?,
        })
    }

    fn extract_item_from_syn(&mut self, item: &mut ItemIdent) -> Result<(), Diagnostic> {
        let mut attr_idxs = Vec::new();
        let mut items = Vec::new();
        let mut cfgs = Vec::new();
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
            match name.to_string().as_str() {
                "pyfunction" => {
                    assert!(item.typ == ItemType::Fn);
                    attr_idxs.push(i);
                    items.push((
                        Self::extract_function(item.ident, into_nested()?)?,
                        meta_span,
                    ));
                }
                "pyattr" => {
                    attr_idxs.push(i);
                    match item.typ {
                        ItemType::Fn => {
                            items.push((
                                Self::extract_evaluated_attr(item.ident, into_nested()?)?,
                                meta_span,
                            ));
                        }
                        ItemType::Const => {
                            items.push((
                                Self::extract_const_attr(item.ident, into_nested()?)?,
                                meta_span,
                            ));
                        }
                        _ => unreachable!(),
                    }
                }
                "pyclass" => {
                    assert!(item.typ == ItemType::Struct);
                    items.push((Self::extract_class(item.ident, into_nested()?)?, meta_span));
                }
                "pystruct_sequence" => {
                    assert!(item.typ == ItemType::Struct);
                    items.push((
                        Self::extract_struct_sequence(item.ident, into_nested()?)?,
                        meta_span,
                    ));
                }
                "cfg" => {
                    cfgs.push(meta);
                    continue;
                }
                _ => {
                    continue;
                }
            };
        }
        for (item, meta) in items {
            self.add_item(item, cfgs.clone(), meta)?;
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

fn extract_module_items(
    mut items: Vec<ItemIdent>,
    module_name: &str,
) -> Result<TokenStream2, Diagnostic> {
    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    let mut module = Module::default();

    for item in items.iter_mut() {
        push_diag_result!(diagnostics, module.extract_item_from_syn(item),);
    }

    let functions = module
        .items
        .into_iter()
        .map(|((_name, cfgs), item)| match item {
            ModuleItem::Function {
                item_ident,
                py_name,
            } => {
                let new_func = quote_spanned!(
                    item_ident.span() =>
                        vm.ctx.new_function_named(#item_ident, #module_name.to_owned(), #py_name.to_owned()));
                quote! {
                    #( #[ #cfgs ])*
                    vm.__module_set_attr(&module, #py_name, #new_func).unwrap();
                }
            }
            ModuleItem::EvaluatedAttr {
                item_ident,
                py_name,
            } => {
                let new_attr = quote_spanned!(
                    item_ident.span() =>
                        vm.new_pyobj(#item_ident(vm)));
                quote! {
                    #( #[ #cfgs ])*
                    vm.__module_set_attr(&module, #py_name, #new_attr).unwrap();
                }
            }
            ModuleItem::ConstAttr {
                item_ident,
                py_name,
            } => {
                let new_attr = quote_spanned!(
                    item_ident.span() =>
                        vm.new_pyobj(#item_ident));
                quote! {
                    #( #[ #cfgs ])*
                    vm.__module_set_attr(&module, #py_name, #new_attr).unwrap();
                }
            }
            ModuleItem::Class {
                item_ident,
                py_name,
            } => {
                let new_class = quote_spanned!(
                    item_ident.span() =>
                        #item_ident::make_class(&vm.ctx));
                quote! {
                    #( #[ #cfgs ])*
                    vm.__module_set_attr(&module, #py_name, #new_class).unwrap();
                }
            }
        });

    Diagnostic::from_vec(diagnostics)?;

    Ok(quote! {
        #(#functions)*
    })
}

pub fn impl_pymodule(attr: AttributeArgs, item: Item) -> Result<TokenStream2, Diagnostic> {
    let mut module = match item {
        Item::Mod(m) => m,
        other => bail_span!(other, "#[pymodule] can only be on a module declaration"),
    };
    let module_name = def_to_name(&module.ident, "pymodule", attr)?;

    let (_, content) = match module.content.as_mut() {
        Some(c) => c,
        None => bail_span!(
            module,
            "#[pymodule] can only be on a module declaration with body"
        ),
    };

    let items = content
        .iter_mut()
        .filter_map(|item| match item {
            Item::Fn(syn::ItemFn { attrs, sig, .. }) => Some(ItemIdent {
                typ: ItemType::Fn,
                attrs,
                ident: &sig.ident,
            }),
            Item::Struct(syn::ItemStruct { attrs, ident, .. }) => Some(ItemIdent {
                typ: ItemType::Struct,
                attrs,
                ident,
            }),
            Item::Enum(syn::ItemEnum { attrs, ident, .. }) => Some(ItemIdent {
                typ: ItemType::Enum,
                attrs,
                ident,
            }),
            Item::Const(syn::ItemConst { attrs, ident, .. }) => Some(ItemIdent {
                typ: ItemType::Const,
                attrs,
                ident,
            }),
            _ => None,
        })
        .collect();

    let extend_mod = extract_module_items(items, &module_name)?;
    content.extend(vec![
        parse_quote! {
            pub(crate) const MODULE_NAME: &str = #module_name;
        },
        parse_quote! {
            pub(crate) fn extend_module(
                vm: &::rustpython_vm::vm::VirtualMachine,
                module: &::rustpython_vm::pyobject::PyObjectRef,
            ) {
                #extend_mod
            }
        },
        parse_quote! {
            #[allow(dead_code)]
            pub(crate) fn make_module(
                vm: &::rustpython_vm::vm::VirtualMachine
            ) -> ::rustpython_vm::pyobject::PyObjectRef {
                let module = vm.new_module(MODULE_NAME, vm.ctx.new_dict());
                extend_module(vm, &module);
                module
            }
        },
    ]);

    Ok(module.into_token_stream())
}
