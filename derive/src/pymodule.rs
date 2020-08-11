use super::Diagnostic;
use crate::util::{
    def_to_name, meta_into_nesteds, optional_attribute_arg, ItemIdent, ItemMeta, ItemType,
};
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{quote, quote_spanned, ToTokens};
use std::collections::HashMap;
use syn::{parse_quote, spanned::Spanned, AttributeArgs, Ident, Item, Meta, NestedMeta};

struct Module {
    name: String,
    items: HashMap<(String, Vec<Meta>), ModuleItem>,
}

#[derive(PartialEq, Eq, Hash)]
enum ModuleItem {
    Function {
        item_ident: Ident,
        py_name: String,
    },
    EvaluatedAttr {
        item_ident: Ident,
        py_name: String,
    },
    ConstAttr {
        item_ident: Ident,
        py_name: String,
    },
    ClassAttr {
        item_ident: Ident,
        py_name: Option<String>,
    },
    Class {
        item_ident: Ident,
        py_name: String,
    },
}

impl ModuleItem {
    fn name(&self) -> String {
        use ModuleItem::*;
        match self {
            Function { py_name, .. } => py_name.clone(),
            EvaluatedAttr { py_name, .. } => py_name.clone(),
            ConstAttr { py_name, .. } => py_name.clone(),
            ClassAttr { py_name, .. } => py_name.clone().unwrap_or_else(String::new),
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

    fn extract_class_attr(
        ident: &Ident,
        nesteds: Vec<NestedMeta>,
    ) -> Result<ModuleItem, Diagnostic> {
        let item_meta =
            ItemMeta::from_nested_meta("pyattr", &ident, &nesteds, ItemMeta::SIMPLE_NAMES)?;
        Ok(ModuleItem::ClassAttr {
            item_ident: ident.clone(),
            py_name: item_meta.optional_name(),
        })
    }

    fn extract_item_from_syn(&mut self, item: &mut ItemIdent) -> Result<(), Diagnostic> {
        let mut attr_idxs = Vec::new();
        let mut items = Vec::new();
        let mut cfgs = Vec::new();
        let mut has_class = false;
        for (i, attr) in item.attrs.iter_mut().enumerate() {
            let meta = attr.parse_meta()?;
            let meta_span = meta.span();
            let name = match meta.path().get_ident() {
                Some(name) => name.clone(),
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
                        ItemType::Struct => {
                            if has_class {
                                return Err(err_span!(
                                    meta,
                                    "#[pyattr] must be placed on top of #[pyclass] or #[pystruct_sequence]",
                                ));
                            }
                            items.push((
                                Self::extract_class_attr(item.ident, into_nested()?)?,
                                meta_span,
                            ));
                        }
                        _ => unreachable!(),
                    }
                }
                attr_name @ "pyclass" | attr_name @ "pystruct_sequence" => {
                    assert!(item.typ == ItemType::Struct);

                    let nested_or = into_nested();
                    let mut metalist = match &meta {
                        Meta::Path(path) => parse_quote!(#path()),
                        Meta::List(metalist) => metalist.clone(),
                        _ => unreachable!(),
                    };

                    if let Ok(nested) = &nested_or {
                        if optional_attribute_arg("py..", "module", &nested)?.is_none() {
                            let module_name = &self.name;
                            metalist.nested.push(parse_quote! {module = #module_name});
                            *attr = parse_quote!(#[#metalist]);
                        }
                    }

                    if has_class {
                        Err(err_span!(
                            meta,
                            "#[{}] in module cannot be duplicated",
                            attr_name,
                        ))
                    } else if !items
                        .iter()
                        .all(|(item, _span)| matches!(item, ModuleItem::ClassAttr{..}))
                    {
                        Err(err_span!(
                            meta,
                            "#[{}] is allowed to have only #[pyattr] as siblings",
                            attr_name,
                        ))
                    } else {
                        has_class = true;
                        Ok(())
                    }?;

                    let class = match attr_name {
                        "pyclass" => Self::extract_class(item.ident, nested_or?)?,
                        "pystruct_sequence" => {
                            // TODO: validate pystruct_sequence doesn't have module
                            Self::extract_struct_sequence(item.ident, nested_or?)?
                        }
                        _ => unreachable!(),
                    };

                    if items.is_empty() {
                        items.push((class, meta_span));
                    } else {
                        for (attr, _) in items.iter_mut() {
                            match attr {
                                ModuleItem::ClassAttr { py_name, .. } => {
                                    if py_name.is_none() {
                                        *py_name = Some(class.name());
                                    }
                                }
                                _ => unreachable!(),
                            }
                        }
                    }
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
        // TODO: Add module to pystruct_sequence
        Ok(())
    }
}

fn extract_module_items(
    mut items: Vec<ItemIdent>,
    module_name: &str,
) -> Result<TokenStream2, Diagnostic> {
    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    let mut module = Module {
        name: module_name.to_owned(),
        items: Default::default(),
    };

    for item in items.iter_mut() {
        push_diag_result!(diagnostics, module.extract_item_from_syn(item),);
    }

    let items = module
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
            class => {
                let (item_ident, py_name) = match class {
                    ModuleItem::ClassAttr {
                        item_ident,
                        py_name: Some(py_name),
                    }   => (item_ident, py_name),
                    ModuleItem::Class {
                        item_ident,
                        py_name,
                    }   => (item_ident, py_name),
                    _ => unreachable!()
                };
                let new_class = quote_spanned!(
                    item_ident.span() =>
                        #item_ident::make_class(&vm.ctx));
                quote! {
                    #( #[ #cfgs ])*
                    {
                        let new_class = #new_class;
                        new_class.set_str_attr("__module__", vm.ctx.new_str(#module_name));
                        vm.__module_set_attr(&module, #py_name, new_class).unwrap();
                    }
                }
            }

        });

    Diagnostic::from_vec(diagnostics)?;

    Ok(quote! {
        #(#items)*
    })
}

pub fn impl_pymodule(attr: AttributeArgs, item: Item) -> Result<TokenStream2, Diagnostic> {
    let mut module = match item {
        Item::Mod(m) => m,
        other => bail_span!(other, "#[pymodule] can only be on a module declaration"),
    };
    let module_name = def_to_name("pymodule", &module.ident, &attr)?;

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
