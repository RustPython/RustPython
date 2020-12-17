use crate::error::Diagnostic;
use crate::util::{
    iter_use_idents, pyclass_ident_and_attrs, AttributeExt, ClassItemMeta, ContentItem,
    ContentItemInner, ErrorVec, ItemMeta, ItemNursery, SimpleItemMeta, ALL_ALLOWED_NAMES,
};
use proc_macro2::TokenStream;
use quote::{quote, quote_spanned, ToTokens};
use syn::{parse_quote, spanned::Spanned, Attribute, AttributeArgs, Ident, Item, Result};
use syn_ext::ext::*;

#[derive(Default)]
struct ModuleContext {
    name: String,
    module_extend_items: ItemNursery,
    errors: Vec<syn::Error>,
}

pub fn impl_pymodule(
    attr: AttributeArgs,
    module_item: Item,
) -> std::result::Result<TokenStream, Diagnostic> {
    let mut module_item = match module_item {
        Item::Mod(m) => m,
        other => bail_span!(other, "#[pymodule] can only be on a full module"),
    };
    let fake_ident = Ident::new("pymodule", module_item.span());
    let module_meta =
        SimpleItemMeta::from_nested(module_item.ident.clone(), fake_ident, attr.into_iter())?;

    // generation resources
    let mut context = ModuleContext {
        name: module_meta.simple_name()?,
        ..Default::default()
    };
    let items = module_item.items_mut().ok_or_else(|| {
        module_meta.new_meta_error("requires actual module, not a module declaration")
    })?;

    // collect to context
    for item in items.iter_mut() {
        let r = item.try_split_attr_mut(|attrs, item| {
            let (pyitems, cfgs) = attrs_to_module_items(&attrs, new_module_item)?;
            for pyitem in pyitems.iter().rev() {
                let r = pyitem.gen_module_item(ModuleItemArgs {
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

    // append additional items
    let module_name = context.name.as_str();
    let module_extend_items = context.module_extend_items;
    items.extend(iter_chain![
        parse_quote! {
            pub(crate) const MODULE_NAME: &'static str = #module_name;
        },
        parse_quote! {
            pub(crate) fn extend_module(
                vm: &::rustpython_vm::VirtualMachine,
                module: &::rustpython_vm::pyobject::PyObjectRef,
            ) {
                #module_extend_items
            }
        },
        parse_quote! {
            #[allow(dead_code)]
            pub(crate) fn make_module(
                vm: &::rustpython_vm::VirtualMachine
            ) -> ::rustpython_vm::pyobject::PyObjectRef {
                let module = vm.new_module(MODULE_NAME, vm.ctx.new_dict());
                extend_module(vm, &module);
                module
            }
        },
    ]);

    Ok(if let Some(error) = context.errors.into_error() {
        let error = Diagnostic::from(error);
        quote! {
            #module_item
            #error
        }
    } else {
        module_item.into_token_stream()
    })
}

fn new_module_item(
    index: usize,
    attr_name: String,
    pyattrs: Option<Vec<usize>>,
) -> Box<dyn ModuleItem> {
    assert!(ALL_ALLOWED_NAMES.contains(&attr_name.as_str()));
    match attr_name.as_str() {
        "pyfunction" => Box::new(FunctionItem {
            inner: ContentItemInner { index, attr_name },
        }),
        "pyattr" => Box::new(AttributeItem {
            inner: ContentItemInner { index, attr_name },
        }),
        "pyclass" => Box::new(ClassItem {
            inner: ContentItemInner { index, attr_name },
            pyattrs: pyattrs.unwrap_or_else(Vec::new),
        }),
        other => unreachable!("#[pymodule] doesn't accept #[{}]", other),
    }
}

fn attrs_to_module_items<F, R>(attrs: &[Attribute], new_item: F) -> Result<(Vec<R>, Vec<Attribute>)>
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
            if attr_name != "pyclass" {
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

/// #[pyclass]
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
    item: &'a Item,
    attrs: &'a mut Vec<Attribute>,
    context: &'a mut ModuleContext,
    cfgs: &'a [Attribute],
}

impl<'a> ModuleItemArgs<'a> {
    fn module_name(&'a self) -> &'a str {
        self.context.name.as_str()
    }
}

trait ModuleItem: ContentItem {
    fn gen_module_item(&self, args: ModuleItemArgs<'_>) -> Result<()>;
}

impl ModuleItem for FunctionItem {
    fn gen_module_item(&self, args: ModuleItemArgs<'_>) -> Result<()> {
        let ident = match args.item {
            Item::Fn(syn::ItemFn { sig, .. }) => sig.ident.clone(),
            other => return Err(self.new_syn_error(other.span(), "can only be on a function")),
        };

        let item_attr = args.attrs.remove(self.index());
        let item_meta = SimpleItemMeta::from_attr(ident.clone(), &item_attr)?;

        let py_name = item_meta.simple_name()?;
        let item = {
            let doc = args.attrs.doc().map_or_else(
                TokenStream::new,
                |doc| quote!(.with_doc(#doc.to_owned(), &vm.ctx)),
            );
            let module = args.module_name();
            let new_func = quote_spanned!(ident.span()=>
                vm.ctx.make_funcdef(#py_name, #ident)
                    #doc
                    .into_function()
                    .with_module(vm.ctx.new_str(#module.to_owned()))
                    .build(&vm.ctx)
            );
            quote! {
                vm.__module_set_attr(&module, #py_name, #new_func).unwrap();
            }
        };

        args.context
            .module_extend_items
            .add_item(py_name, args.cfgs.to_vec(), item)?;
        Ok(())
    }
}

impl ModuleItem for ClassItem {
    fn gen_module_item(&self, args: ModuleItemArgs<'_>) -> Result<()> {
        let (ident, _) = pyclass_ident_and_attrs(&args.item)?;
        let (module_name, class_name) = {
            let class_attr = &mut args.attrs[self.inner.index];
            if self.pyattrs.is_empty() {
                // check noattr before ClassItemMeta::from_attr
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

            let class_meta = ClassItemMeta::from_attr(ident.clone(), class_attr)?;
            let module_name = args.context.name.clone();
            class_attr.fill_nested_meta("module", || {
                parse_quote! {module = #module_name}
            })?;
            let class_name = class_meta.class_name()?;
            (module_name, class_name)
        };
        for attr_index in self.pyattrs.iter().rev() {
            let mut loop_unit = || {
                let attr_attr = args.attrs.remove(*attr_index);
                let item_meta = SimpleItemMeta::from_attr(ident.clone(), &attr_attr)?;

                let py_name = item_meta
                    .optional_name()
                    .unwrap_or_else(|| class_name.clone());
                let new_class = quote_spanned!(ident.span() =>
                    <#ident as ::rustpython_vm::pyobject::PyClassImpl>::make_class(&vm.ctx);
                );
                let item = quote! {
                    let new_class = #new_class;
                    new_class.set_str_attr("__module__", vm.ctx.new_str(#module_name));
                    vm.__module_set_attr(&module, #py_name, new_class).unwrap();
                };

                args.context
                    .module_extend_items
                    .add_item(py_name, args.cfgs.to_vec(), item)?;
                Ok(())
            };
            let r = loop_unit();
            args.context.errors.ok_or_push(r);
        }
        Ok(())
    }
}

impl ModuleItem for AttributeItem {
    fn gen_module_item(&self, args: ModuleItemArgs<'_>) -> Result<()> {
        let cfgs = args.cfgs.to_vec();
        let attr = args.attrs.remove(self.index());
        let get_py_name = |attr: &Attribute, ident: &Ident| -> Result<_> {
            let item_meta = SimpleItemMeta::from_attr(ident.clone(), attr)?;
            let py_name = item_meta.simple_name()?;
            Ok(py_name)
        };
        let (py_name, tokens) = match args.item {
            Item::Fn(syn::ItemFn { sig, .. }) => {
                let ident = &sig.ident;
                let py_name = get_py_name(&attr, &ident)?;
                (
                    py_name.clone(),
                    quote! {
                        vm.__module_set_attr(&module, #py_name, vm.new_pyobj(#ident(vm))).unwrap();
                    },
                )
            }
            Item::Const(syn::ItemConst { ident, .. }) => {
                let py_name = get_py_name(&attr, &ident)?;
                (
                    py_name.clone(),
                    quote! {
                        vm.__module_set_attr(&module, #py_name, vm.new_pyobj(#ident)).unwrap();
                    },
                )
            }
            Item::Use(item) => {
                return iter_use_idents(item, |ident, is_unique| {
                    let item_meta = SimpleItemMeta::from_attr(ident.clone(), &attr)?;
                    let py_name = if is_unique {
                        item_meta.simple_name()?
                    } else if item_meta.optional_name().is_some() {
                        // this check actually doesn't need to be placed in loop
                        return Err(self.new_syn_error(
                            ident.span(),
                            "`name` attribute is not allowed for multiple use items",
                        ));
                    } else {
                        ident.to_string()
                    };
                    let tokens = quote! {
                        vm.__module_set_attr(&module, #py_name, vm.new_pyobj(#ident)).unwrap();
                    };
                    args.context
                        .module_extend_items
                        .add_item(py_name, cfgs.clone(), tokens)?;
                    Ok(())
                });
            }
            other => {
                return Err(
                    self.new_syn_error(other.span(), "can only be on a function, const and use")
                )
            }
        };

        args.context
            .module_extend_items
            .add_item(py_name, cfgs, tokens)?;

        Ok(())
    }
}
