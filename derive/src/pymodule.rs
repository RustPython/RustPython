use crate::error::Diagnostic;
use crate::util::{
    AttributeExt, ClassItemMeta, ContentItem, ContentItemInner, ItemMeta, ItemNursery,
    SimpleItemMeta, ALL_ALLOWED_NAMES,
};
use proc_macro2::TokenStream;
use quote::{quote, quote_spanned, ToTokens};
use syn::{parse_quote, spanned::Spanned, Attribute, AttributeArgs, Ident, Item, Result, UseTree};
use syn_ext::ext::*;

struct Module {
    name: String,
    items: ItemNursery,
}

pub fn impl_pymodule(
    attr: AttributeArgs,
    module_item: Item,
) -> std::result::Result<TokenStream, Diagnostic> {
    let mut module_item = match module_item {
        Item::Mod(m) => m,
        other => bail_span!(other, "#[pymodule] can only be on a module declaration"),
    };
    let module_meta =
        SimpleItemMeta::from_nested("pymodule", &module_item.ident, attr.into_iter())?;
    let mut module_context = Module {
        name: module_meta.simple_name()?,
        items: ItemNursery::default(),
    };
    let content = module_item.unbraced_content_mut()?;

    for item in content.iter_mut() {
        let (attrs, ident) = if let Some(v) = item_declaration(item) {
            v
        } else {
            continue;
        };
        let (pyitems, cfgs) = attrs_to_items(attrs, new_item)?;
        for pyitem in pyitems.iter().rev() {
            pyitem.gen_module_item(ModuleItemArgs {
                ident: ident.clone(),
                item,
                module: &mut module_context,
                cfgs: cfgs.as_slice(),
            })?;
        }
    }

    let module_name = module_context.name.as_str();
    let items = module_context.items;
    content.extend(vec![
        parse_quote! {
            pub(crate) const MODULE_NAME: &'static str = #module_name;
        },
        parse_quote! {
            pub(crate) fn extend_module(
                vm: &::rustpython_vm::vm::VirtualMachine,
                module: &::rustpython_vm::pyobject::PyObjectRef,
            ) {
                #items
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

    Ok(module_item.into_token_stream())
}

fn item_declaration(item: &syn::Item) -> Option<(&[syn::Attribute], Ident)> {
    let (attrs, ident) = match &item {
        Item::Fn(i @ syn::ItemFn { .. }) => (i.attrs.as_slice(), &i.sig.ident),
        Item::Struct(syn::ItemStruct { attrs, ident, .. }) => (attrs.as_slice(), ident),
        Item::Enum(syn::ItemEnum { attrs, ident, .. }) => (attrs.as_slice(), ident),
        Item::Const(syn::ItemConst { attrs, ident, .. }) => (attrs.as_slice(), ident),
        Item::Use(syn::ItemUse { attrs, tree, .. }) => {
            let ident = match tree {
                UseTree::Path(path) => match &*path.tree {
                    UseTree::Name(name) => &name.ident,
                    UseTree::Rename(rename) => &rename.rename,
                    _ => return None,
                },
                _ => return None,
            };
            (attrs.as_slice(), ident)
        }
        _ => return None,
    };
    Some((attrs, ident.clone()))
}

fn new_item(index: usize, attr_name: String, pyattrs: Option<Vec<usize>>) -> Box<dyn ModuleItem> {
    assert!(ALL_ALLOWED_NAMES.contains(&attr_name.as_str()));
    match attr_name.as_str() {
        "pyfunction" => Box::new(FunctionItem {
            inner: ContentItemInner { index, attr_name },
        }),
        "pyattr" => Box::new(AttributeItem {
            inner: ContentItemInner { index, attr_name },
        }),
        "pystruct_sequence" | "pyclass" => Box::new(ClassItem {
            inner: ContentItemInner { index, attr_name },
            pyattrs: pyattrs.unwrap_or_else(Vec::new),
        }),
        other => unreachable!("#[pymodule] doesn't accept #[{}]", other),
    }
}

fn attrs_to_items<F, R>(attrs: &[Attribute], new_item: F) -> Result<(Vec<R>, Vec<Attribute>)>
where
    F: Fn(usize, String, Option<Vec<usize>>) -> R,
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

    let mut closed = false;
    let mut pyattrs = Vec::new();
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
        } else if closed {
            return Err(syn::Error::new_spanned(
                attr,
                "Only one #[pyattr] annotated #[py*] item can exist",
            ));
        }

        if attr_name == "pyattr" {
            if !result.is_empty() {
                return Err(syn::Error::new_spanned(
                    attr,
                    "#[pyattr] must be placed on top of other #[py*] items",
                ));
            }
            pyattrs.push((i, attr_name));
            continue;
        }

        if pyattrs.is_empty() {
            result.push(new_item(i, attr_name, None));
        } else {
            if !["pyclass", "pystruct_sequence"].contains(&attr_name.as_str()) {
                return Err(syn::Error::new_spanned(
                    attr,
                    "#[pyattr] #[pyclass] is the only supported composition",
                ));
            }
            let pyattr_indexes = pyattrs.iter().map(|(i, _)| i).copied().collect();
            result.push(new_item(i, attr_name, Some(pyattr_indexes)));
            pyattrs = Vec::new();
            closed = true;
        }
    }
    for (index, attr_name) in pyattrs {
        assert!(!closed);
        result.push(new_item(index, attr_name, None));
    }
    Ok((result, cfgs))
}

/// #[pyfunction]
struct FunctionItem {
    inner: ContentItemInner,
}

/// #[pyclass] or #[pystruct_sequence]
struct ClassItem {
    inner: ContentItemInner,
    pyattrs: Vec<usize>,
}

/// #[pyattr]
struct AttributeItem {
    inner: ContentItemInner,
}

impl ContentItem for FunctionItem {
    fn inner(&self) -> &ContentItemInner {
        &self.inner
    }
}

impl ContentItem for ClassItem {
    fn inner(&self) -> &ContentItemInner {
        &self.inner
    }
}

impl ContentItem for AttributeItem {
    fn inner(&self) -> &ContentItemInner {
        &self.inner
    }
}

struct ModuleItemArgs<'a> {
    ident: Ident,
    item: &'a mut Item,
    module: &'a mut Module,
    cfgs: &'a [Attribute],
}

impl<'a> ModuleItemArgs<'a> {
    fn module_name(&'a self) -> &'a str {
        self.module.name.as_str()
    }
    fn with_quote_args<F>(&self, f: F) -> TokenStream
    where
        F: Fn(&Ident, &str) -> TokenStream,
    {
        f(&self.ident, self.module_name())
    }
}

trait ModuleItem: ContentItem {
    fn gen_module_item(&self, args: ModuleItemArgs<'_>) -> Result<()>;
}

impl ModuleItem for FunctionItem {
    fn gen_module_item(&self, args: ModuleItemArgs<'_>) -> Result<()> {
        let attrs = match args.item {
            Item::Fn(syn::ItemFn { ref mut attrs, .. }) => attrs,
            other => return Err(self.new_syn_error(other.span(), "can only be on a function")),
        };
        let item_attr = attrs.remove(self.index());
        let item_meta = SimpleItemMeta::from_nested(
            "pyfunction",
            &args.ident,
            item_attr.promoted_nested()?.into_iter(),
        )?;

        let py_name = item_meta.simple_name()?;
        let item = args.with_quote_args(|ident, module| {
            let new_func = quote_spanned!(
                args.ident.span() => vm.ctx.new_function_named(#ident, #module.to_owned(), #py_name.to_owned())
            );
            quote! {
                vm.__module_set_attr(&module, #py_name, #new_func).unwrap();
            }
        });

        args.module
            .items
            .add_item(py_name, args.cfgs.to_vec(), item)?;
        Ok(())
    }
}

impl ModuleItem for ClassItem {
    fn gen_module_item(&self, args: ModuleItemArgs<'_>) -> Result<()> {
        let attrs = match args.item {
            Item::Struct(syn::ItemStruct { ref mut attrs, .. }) => attrs,
            Item::Enum(syn::ItemEnum { ref mut attrs, .. }) => attrs,
            other => {
                return Err(
                    self.new_syn_error(other.span(), "can only be on a struct or enum declaration")
                );
            }
        };
        let (module_name, class_name) = {
            let class_attr = &mut attrs[self.inner.index];
            if self.pyattrs.is_empty() {
                // check noattr before ClassItemMeta::from_nested
                let noattr = class_attr.try_remove_name("noattr")?;
                if noattr.is_none() {
                    return Err(syn::Error::new_spanned(
                        class_attr,
                        format!(
                            "#[{name}] requires #[pyattr] to be a module attribute. \
                         To keep it free type, try #[{name}(noattr)]",
                            name = self.attr_name()
                        ),
                    ));
                }
            }
            let static_name = match self.attr_name() {
                "pyclass" => "pyclass",
                "pystruct_sequence" => "pystruct_sequence",
                _ => unreachable!(),
            };
            let class_meta = ClassItemMeta::from_nested(
                static_name,
                &args.ident,
                class_attr.promoted_nested()?.into_iter(),
            )?;
            let module_name = args.module.name.clone();
            class_attr.fill_nested_meta("module", || {
                parse_quote! {module = #module_name}
            })?;
            let class_name = class_meta.class_name()?;
            (module_name, class_name)
        };
        for attr_index in self.pyattrs.iter().rev() {
            let attr_attr = attrs.remove(*attr_index);
            let nested = attr_attr.promoted_nested()?;
            let item_meta = SimpleItemMeta::from_nested("pyattr", &args.ident, nested.into_iter())?;

            let py_name = item_meta
                .optional_name()
                .unwrap_or_else(|| class_name.clone());
            let ident = &args.ident;
            let new_class = quote_spanned!(ident.span() =>
                #ident::make_class(&vm.ctx);
            );
            let item = quote! {
                let new_class = #new_class;
                new_class.set_str_attr("__module__", vm.ctx.new_str(#module_name));
                vm.__module_set_attr(&module, #py_name, new_class).unwrap();
            };

            args.module
                .items
                .add_item(py_name.clone(), args.cfgs.to_vec(), item)?;
        }
        Ok(())
    }
}

impl ModuleItem for AttributeItem {
    fn gen_module_item(&self, args: ModuleItemArgs<'_>) -> Result<()> {
        let ident = &args.ident;
        let get_py_name = |attrs: &mut Vec<Attribute>| -> Result<_> {
            let nested = attrs[self.inner.index].promoted_nested()?;
            let item_meta = SimpleItemMeta::from_nested("pyattr", &ident, nested.into_iter())?;
            let py_name = item_meta.simple_name()?;
            Ok(py_name)
        };
        let (attrs, py_name, tokens) = match args.item {
            Item::Fn(syn::ItemFn { ref mut attrs, .. }) => {
                let py_name = get_py_name(attrs)?;
                (
                    attrs,
                    py_name.clone(),
                    quote! {
                        vm.__module_set_attr(&module, #py_name, vm.new_pyobj(#ident(vm))).unwrap();
                    },
                )
            }
            Item::Const(syn::ItemConst { ref mut attrs, .. }) => {
                let py_name = get_py_name(attrs)?;
                (
                    attrs,
                    py_name.clone(),
                    quote! {
                        vm.__module_set_attr(&module, #py_name, vm.new_pyobj(#ident)).unwrap();
                    },
                )
            }
            Item::Use(syn::ItemUse { ref mut attrs, .. }) => {
                let py_name = get_py_name(attrs)?;
                (
                    attrs,
                    py_name.clone(),
                    quote! {
                        vm.__module_set_attr(&module, #py_name, vm.new_pyobj(#ident)).unwrap();
                    },
                )
            }
            other => {
                return Err(
                    self.new_syn_error(other.span(), "can only be on a function, const and use")
                )
            }
        };
        attrs.remove(self.index());

        args.module
            .items
            .add_item(py_name, args.cfgs.to_vec(), tokens)?;

        Ok(())
    }
}
