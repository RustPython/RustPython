use super::Diagnostic;
use crate::util::{def_to_name, ItemMeta};
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{quote, quote_spanned};
use std::collections::HashSet;
use syn::{
    parse_quote, spanned::Spanned, Attribute, AttributeArgs, Ident, Item, Meta, NestedMeta,
    Signature,
};

fn meta_to_vec(meta: Meta) -> Result<Vec<NestedMeta>, Meta> {
    match meta {
        Meta::Path(_) => Ok(Vec::new()),
        Meta::List(list) => Ok(list.nested.into_iter().collect()),
        Meta::NameValue(_) => Err(meta),
    }
}

#[derive(Default)]
struct Module {
    items: HashSet<ModuleItem>,
}

#[derive(PartialEq, Eq, Hash)]
enum ModuleItem {
    Function { item_ident: Ident, py_name: String },
}

impl Module {
    fn add_item(&mut self, item: ModuleItem, span: Span) -> Result<(), Diagnostic> {
        if self.items.insert(item) {
            Ok(())
        } else {
            Err(Diagnostic::span_error(
                span,
                "Duplicate #[py*] attribute on pyimpl".to_owned(),
            ))
        }
    }

    fn extract_function(sig: &Signature, meta: Meta) -> Result<ModuleItem, Diagnostic> {
        let nesteds = meta_to_vec(meta).map_err(|meta| {
            err_span!(
                meta,
                "#[pyfunction = \"...\"] cannot be a name/value, you probably meant \
                 #[pyfunction(name = \"...\")]",
            )
        })?;

        let item_meta =
            ItemMeta::from_nested_meta("pyfunction", sig, &nesteds, ItemMeta::SIMPLE_NAMES)?;
        Ok(ModuleItem::Function {
            item_ident: sig.ident.clone(),
            py_name: item_meta.simple_name()?,
        })
    }

    fn extract_item_from_syn(
        &mut self,
        attrs: &mut Vec<Attribute>,
        sig: &Signature,
    ) -> Result<(), Diagnostic> {
        let mut attr_idxs = Vec::new();
        for (i, meta) in attrs
            .iter()
            .filter_map(|attr| attr.parse_meta().ok())
            .enumerate()
        {
            let meta_span = meta.span();
            let name = match meta.path().get_ident() {
                Some(name) => name,
                None => continue,
            };
            if name == "pyfunction" {
                self.add_item(Self::extract_function(sig, meta)?, meta_span)?;
            } else {
                continue;
            }
            attr_idxs.push(i);
        }
        let mut i = 0;
        let mut attr_idxs = &*attr_idxs;
        attrs.retain(|_| {
            let drop = attr_idxs.first().copied() == Some(i);
            if drop {
                attr_idxs = &attr_idxs[1..];
            }
            i += 1;
            !drop
        });
        for (i, idx) in attr_idxs.iter().enumerate() {
            attrs.remove(idx - i);
        }
        Ok(())
    }
}

struct ItemSig<'a> {
    attrs: &'a mut Vec<Attribute>,
    sig: &'a Signature,
}

fn extract_module_items(mut items: Vec<ItemSig>) -> Result<TokenStream2, Diagnostic> {
    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    let mut class = Module::default();

    for item in items.iter_mut() {
        push_diag_result!(
            diagnostics,
            class.extract_item_from_syn(&mut item.attrs, item.sig),
        );
    }

    let functions = class.items.into_iter().map(|item| match item {
        ModuleItem::Function {
            item_ident,
            py_name,
        } => {
            let new_func = quote_spanned!(item_ident.span()=> .new_function(#item_ident));
            quote! {
                vm.__module_set_attr(&module, #py_name, vm.ctx#new_func).unwrap();
            }
        }
    });

    Diagnostic::from_vec(diagnostics)?;

    Ok(quote! {
        #(#functions)*
    })
}

pub fn impl_pymodule(attr: AttributeArgs, item: Item) -> Result<TokenStream2, Diagnostic> {
    match item {
        Item::Mod(mut module) => {
            let module_name = def_to_name(&module.ident, "pymodule", attr)?;

            let content = &mut module.content.as_mut().unwrap().1;
            let items = content
                .iter_mut()
                .filter_map(|item| match item {
                    syn::Item::Fn(syn::ItemFn {
                        attrs,
                        vis: _vis,
                        sig,
                        ..
                    }) => Some(ItemSig { attrs, sig }),
                    _ => None,
                })
                .collect();

            let extend_mod = extract_module_items(items)?;
            content.push(parse_quote! {
                pub(crate) fn extend_module(
                    vm: &::rustpython_vm::vm::VirtualMachine,
                    module: &::rustpython_vm::pyobject::PyObjectRef,
                ) {
                    #extend_mod
                }
            });
            content.push(parse_quote! {
                #[allow(dead_code)]
                pub(crate) fn make_module(
                    vm: &::rustpython_vm::vm::VirtualMachine
                ) -> ::rustpython_vm::pyobject::PyObjectRef {
                    let module = vm.new_module(#module_name, vm.ctx.new_dict());
                    extend_module(vm, &module);
                    module
                }
            });

            Ok(quote! {
                #module
            })
        }
        other => bail_span!(other, "#[pymodule] can only be on a module declaration"),
    }
}
