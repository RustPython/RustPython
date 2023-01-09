use crate::error::Diagnostic;
use crate::util::{
    format_doc, iter_use_idents, pyclass_ident_and_attrs, text_signature, AttrItemMeta,
    AttributeExt, ClassItemMeta, ContentItem, ContentItemInner, ErrorVec, ItemMeta, ItemNursery,
    SimpleItemMeta, ALL_ALLOWED_NAMES,
};
use proc_macro2::{Span, TokenStream};
use quote::{quote, quote_spanned, ToTokens};
use std::{collections::HashSet, str::FromStr};
use syn::{parse_quote, spanned::Spanned, Attribute, AttributeArgs, Ident, Item, Result};
use syn_ext::ext::*;

#[derive(Clone, Copy, Eq, PartialEq)]
enum AttrName {
    Function,
    Attr,
    Class,
}

impl std::fmt::Display for AttrName {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let s = match self {
            Self::Function => "pyfunction",
            Self::Attr => "pyattr",
            Self::Class => "pyclass",
        };
        s.fmt(f)
    }
}

impl FromStr for AttrName {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(match s {
            "pyfunction" => Self::Function,
            "pyattr" => Self::Attr,
            "pyclass" => Self::Class,
            s => {
                return Err(s.to_owned());
            }
        })
    }
}

#[derive(Default)]
struct ModuleContext {
    name: String,
    module_extend_items: ItemNursery,
    errors: Vec<syn::Error>,
}

pub fn impl_pymodule(attr: AttributeArgs, module_item: Item) -> Result<TokenStream> {
    let (doc, mut module_item) = match module_item {
        Item::Mod(m) => (m.attrs.doc(), m),
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
        if matches!(item, Item::Impl(_) | Item::Trait(_)) {
            // #[pyclass] implementations
            continue;
        }
        let r = item.try_split_attr_mut(|attrs, item| {
            let (pyitems, cfgs) = attrs_to_module_items(attrs, module_item_new)?;
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
    let module_extend_items = context.module_extend_items.validate()?;
    let doc = doc.or_else(|| {
        crate::doc::Database::shared()
            .try_path(module_name)
            .ok()
            .flatten()
            .map(str::to_owned)
    });
    let doc = if let Some(doc) = doc {
        quote!(Some(#doc))
    } else {
        quote!(None)
    };
    items.extend(iter_chain![
        parse_quote! {
            pub(crate) const MODULE_NAME: &'static str = #module_name;
        },
        parse_quote! {
            pub(crate) const DOC: Option<&'static str> = #doc;
        },
        parse_quote! {
            pub(crate) fn extend_module(
                vm: &::rustpython_vm::VirtualMachine,
                module: &::rustpython_vm::PyObject,
            ) {
                #module_extend_items
            }
        },
        parse_quote! {
            #[allow(dead_code)]
            pub(crate) fn make_module(
                vm: &::rustpython_vm::VirtualMachine
            ) -> ::rustpython_vm::PyObjectRef {
                let module = vm.new_module(MODULE_NAME, vm.ctx.new_dict(), DOC);
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

fn module_item_new(
    index: usize,
    attr_name: AttrName,
    pyattrs: Vec<usize>,
) -> Box<dyn ModuleItem<AttrName = AttrName>> {
    match attr_name {
        AttrName::Function => Box::new(FunctionItem {
            inner: ContentItemInner { index, attr_name },
            pyattrs,
        }),
        AttrName::Attr => Box::new(AttributeItem {
            inner: ContentItemInner { index, attr_name },
            pyattrs,
        }),
        AttrName::Class => Box::new(ClassItem {
            inner: ContentItemInner { index, attr_name },
            pyattrs,
        }),
    }
}

fn attrs_to_module_items<F, R>(attrs: &[Attribute], item_new: F) -> Result<(Vec<R>, Vec<Attribute>)>
where
    F: Fn(usize, AttrName, Vec<usize>) -> R,
{
    let mut cfgs: Vec<Attribute> = Vec::new();
    let mut result = Vec::new();

    let mut iter = attrs.iter().enumerate().peekable();
    while let Some((_, attr)) = iter.peek() {
        // take all cfgs but no py items
        let attr = *attr;
        if let Some(ident) = attr.get_ident() {
            let attr_name = ident.to_string();
            if attr_name == "cfg" {
                cfgs.push(attr.clone());
            } else if ALL_ALLOWED_NAMES.contains(&attr_name.as_str()) {
                break;
            }
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
            bail_span!(attr, "#[py*] items must be placed under `cfgs`")
        }

        let attr_name = match AttrName::from_str(attr_name.as_str()) {
            Ok(name) => name,
            Err(wrong_name) => {
                if !ALL_ALLOWED_NAMES.contains(&wrong_name.as_str()) {
                    continue;
                } else if closed {
                    bail_span!(attr, "Only one #[pyattr] annotated #[py*] item can exist")
                } else {
                    bail_span!(attr, "#[pymodule] doesn't accept #[{}]", wrong_name)
                }
            }
        };

        if attr_name == AttrName::Attr {
            if !result.is_empty() {
                bail_span!(
                    attr,
                    "#[pyattr] must be placed on top of other #[py*] items",
                )
            }
            pyattrs.push(i);
            continue;
        }

        if pyattrs.is_empty() {
            result.push(item_new(i, attr_name, Vec::new()));
        } else {
            match attr_name {
                AttrName::Class | AttrName::Function => {
                    result.push(item_new(i, attr_name, pyattrs.clone()));
                }
                _ => {
                    bail_span!(
                        attr,
                        "#[pyclass] or #[pyfunction] only can follow #[pyattr]",
                    )
                }
            }
            pyattrs.clear();
            closed = true;
        }
    }

    if let Some(last) = pyattrs.pop() {
        assert!(!closed);
        result.push(item_new(last, AttrName::Attr, pyattrs));
    }
    Ok((result, cfgs))
}

/// #[pyfunction]
struct FunctionItem {
    inner: ContentItemInner<AttrName>,
    pyattrs: Vec<usize>,
}

/// #[pyclass]
struct ClassItem {
    inner: ContentItemInner<AttrName>,
    pyattrs: Vec<usize>,
}

/// #[pyattr]
struct AttributeItem {
    inner: ContentItemInner<AttrName>,
    pyattrs: Vec<usize>,
}

impl ContentItem for FunctionItem {
    type AttrName = AttrName;
    fn inner(&self) -> &ContentItemInner<AttrName> {
        &self.inner
    }
}

impl ContentItem for ClassItem {
    type AttrName = AttrName;
    fn inner(&self) -> &ContentItemInner<AttrName> {
        &self.inner
    }
}

impl ContentItem for AttributeItem {
    type AttrName = AttrName;
    fn inner(&self) -> &ContentItemInner<AttrName> {
        &self.inner
    }
}

struct ModuleItemArgs<'a> {
    item: &'a mut Item,
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
        let func = args
            .item
            .function_or_method()
            .map_err(|_| self.new_syn_error(args.item.span(), "can only be on a function"))?;
        let ident = &func.sig().ident;

        let item_attr = args.attrs.remove(self.index());
        let item_meta = SimpleItemMeta::from_attr(ident.clone(), &item_attr)?;

        let py_name = item_meta.simple_name()?;
        let sig_doc = text_signature(func.sig(), &py_name);

        let (tokens, py_names) = {
            let module = args.module_name();
            let doc = args.attrs.doc().or_else(|| {
                crate::doc::Database::shared()
                    .try_module_item(module, &py_name)
                    .ok() // TODO: doc must exist at least one of code or CPython
                    .flatten()
                    .map(str::to_owned)
            });
            let doc = if let Some(doc) = doc {
                format_doc(&sig_doc, &doc)
            } else {
                sig_doc
            };
            let doc = quote!(.with_doc(#doc.to_owned(), &vm.ctx));
            let new_func = quote_spanned!(ident.span()=>
                vm.ctx.make_funcdef(#py_name, #ident)
                    #doc
                    .into_function()
                    .with_module(vm.new_pyobj(#module.to_owned()))
                    .into_ref(&vm.ctx)
            );

            if self.pyattrs.is_empty() {
                (
                    quote_spanned! { ident.span() => {
                        let func = #new_func;
                        vm.__module_set_attr(module, #py_name, func).unwrap();
                    }},
                    vec![py_name],
                )
            } else {
                let mut py_names = HashSet::new();
                py_names.insert(py_name);
                for attr_index in self.pyattrs.iter().rev() {
                    let mut loop_unit = || {
                        let attr_attr = args.attrs.remove(*attr_index);
                        let item_meta = SimpleItemMeta::from_attr(ident.clone(), &attr_attr)?;

                        let py_name = item_meta.simple_name()?;
                        let inserted = py_names.insert(py_name.clone());
                        if !inserted {
                            return Err(self.new_syn_error(
                                ident.span(),
                                &format!(
                                    "`{py_name}` is duplicated name for multiple py* attribute"
                                ),
                            ));
                        }
                        Ok(())
                    };
                    let r = loop_unit();
                    args.context.errors.ok_or_push(r);
                }
                let py_names: Vec<_> = py_names.into_iter().collect();
                (
                    quote_spanned! { ident.span().resolved_at(Span::call_site()) => {
                        let func = #new_func;
                        for name in [#(#py_names,)*] {
                            vm.__module_set_attr(module, name, func.clone()).unwrap();
                        }
                    }},
                    py_names,
                )
            }
        };

        args.context.module_extend_items.add_item(
            ident.clone(),
            py_names,
            args.cfgs.to_vec(),
            tokens,
            10,
        )?;
        Ok(())
    }
}

impl ModuleItem for ClassItem {
    fn gen_module_item(&self, args: ModuleItemArgs<'_>) -> Result<()> {
        let (ident, _) = pyclass_ident_and_attrs(args.item)?;
        let (class_name, class_new) = {
            let class_attr = &mut args.attrs[self.inner.index];
            let no_attr = class_attr.try_remove_name("no_attr")?;
            if self.pyattrs.is_empty() {
                // check no_attr before ClassItemMeta::from_attr
                if no_attr.is_none() {
                    bail_span!(
                        ident,
                        "#[{name}] requires #[pyattr] to be a module attribute. \
                         To keep it free type, try #[{name}(no_attr)]",
                        name = self.attr_name()
                    )
                }
            }
            let no_attr = no_attr.is_some();
            let is_use = matches!(&args.item, syn::Item::Use(_));

            let class_meta = ClassItemMeta::from_attr(ident.clone(), class_attr)?;
            let module_name = args.context.name.clone();
            let module_name = if let Some(class_module_name) = class_meta.module().ok().flatten() {
                class_module_name
            } else {
                class_attr.fill_nested_meta("module", || {
                    parse_quote! {module = #module_name}
                })?;
                module_name
            };
            let class_name = if no_attr && is_use {
                "<NO ATTR>".to_owned()
            } else {
                class_meta.class_name()?
            };
            let class_new = quote_spanned!(ident.span() =>
                let new_class = <#ident as ::rustpython_vm::class::PyClassImpl>::make_class(&vm.ctx);
                new_class.set_attr(rustpython_vm::identifier!(vm, __module__), vm.new_pyobj(#module_name));
            );
            (class_name, class_new)
        };

        let mut py_names = Vec::new();
        for attr_index in self.pyattrs.iter().rev() {
            let mut loop_unit = || {
                let attr_attr = args.attrs.remove(*attr_index);
                let item_meta = SimpleItemMeta::from_attr(ident.clone(), &attr_attr)?;

                let py_name = item_meta
                    .optional_name()
                    .unwrap_or_else(|| class_name.clone());
                py_names.push(py_name);

                Ok(())
            };
            let r = loop_unit();
            args.context.errors.ok_or_push(r);
        }

        let set_attr = match py_names.len() {
            0 => quote! {
                let _ = new_class;  // suppress warning
            },
            1 => {
                let py_name = &py_names[0];
                quote! {
                    vm.__module_set_attr(&module, #py_name, new_class).unwrap();
                }
            }
            _ => quote! {
                for name in [#(#py_names,)*] {
                    vm.__module_set_attr(&module, name, new_class.clone()).unwrap();
                }
            },
        };

        args.context.module_extend_items.add_item(
            ident.clone(),
            py_names,
            args.cfgs.to_vec(),
            quote_spanned! { ident.span() =>
                #class_new
                #set_attr
            },
            0,
        )?;
        Ok(())
    }
}

impl ModuleItem for AttributeItem {
    fn gen_module_item(&self, args: ModuleItemArgs<'_>) -> Result<()> {
        let cfgs = args.cfgs.to_vec();
        let attr = args.attrs.remove(self.index());
        let (ident, py_name, let_obj) = match args.item {
            Item::Fn(syn::ItemFn { sig, block, .. }) => {
                let ident = &sig.ident;
                // If `once` keyword is in #[pyattr],
                // wrapping it with static_cell for preventing it from using it as function
                let attr_meta = AttrItemMeta::from_attr(ident.clone(), &attr)?;
                if attr_meta.inner()._bool("once")? {
                    let stmts = &block.stmts;
                    let return_type = match &sig.output {
                        syn::ReturnType::Default => {
                            unreachable!("#[pyattr] attached function must have return type.")
                        }
                        syn::ReturnType::Type(_, ty) => ty,
                    };
                    let stmt: syn::Stmt = parse_quote! {
                        {
                            rustpython_common::static_cell! {
                                static ERROR: #return_type;
                            }
                            ERROR
                                .get_or_init(|| {
                                    #(#stmts)*
                                })
                                .clone()
                        }
                    };
                    block.stmts = vec![stmt];
                }

                let py_name = attr_meta.simple_name()?;
                (
                    ident.clone(),
                    py_name,
                    quote_spanned! { ident.span() =>
                        let obj = vm.new_pyobj(#ident(vm));
                    },
                )
            }
            Item::Const(syn::ItemConst { ident, .. }) => {
                let item_meta = SimpleItemMeta::from_attr(ident.clone(), &attr)?;
                let py_name = item_meta.simple_name()?;
                (
                    ident.clone(),
                    py_name,
                    quote_spanned! { ident.span() =>
                        let obj = vm.new_pyobj(#ident);
                    },
                )
            }
            Item::Use(item) => {
                if !self.pyattrs.is_empty() {
                    return Err(self
                        .new_syn_error(item.span(), "Only single #[pyattr] is allowed for `use`"));
                }
                let _ = iter_use_idents(item, |ident, is_unique| {
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
                    let tokens = quote_spanned! { ident.span() =>
                        vm.__module_set_attr(module, #py_name, vm.new_pyobj(#ident)).unwrap();
                    };
                    args.context.module_extend_items.add_item(
                        ident.clone(),
                        vec![py_name],
                        cfgs.clone(),
                        tokens,
                        1,
                    )?;
                    Ok(())
                })?;
                return Ok(());
            }
            other => {
                return Err(
                    self.new_syn_error(other.span(), "can only be on a function, const and use")
                )
            }
        };

        let (tokens, py_names) = if self.pyattrs.is_empty() {
            (
                quote_spanned! { ident.span() => {
                    #let_obj
                    vm.__module_set_attr(module, #py_name, obj).unwrap();
                }},
                vec![py_name],
            )
        } else {
            let mut names = vec![py_name];
            for attr_index in self.pyattrs.iter().rev() {
                let mut loop_unit = || {
                    let attr_attr = args.attrs.remove(*attr_index);
                    let item_meta = AttrItemMeta::from_attr(ident.clone(), &attr_attr)?;
                    if item_meta.inner()._bool("once")? {
                        return Err(self.new_syn_error(
                            ident.span(),
                            "#[pyattr(once)] is only allowed for the bottom-most item",
                        ));
                    }

                    let py_name = item_meta.optional_name().ok_or_else(|| {
                        self.new_syn_error(
                            ident.span(),
                            "#[pyattr(name = ...)] is mandatory except for the bottom-most item",
                        )
                    })?;
                    names.push(py_name);
                    Ok(())
                };
                let r = loop_unit();
                args.context.errors.ok_or_push(r);
            }
            (
                quote_spanned! { ident.span() => {
                    #let_obj
                    for name in [(#(#names,)*)] {
                        vm.__module_set_attr(module, name, obj.clone()).unwrap();
                    }
                }},
                names,
            )
        };

        args.context
            .module_extend_items
            .add_item(ident, py_names, cfgs, tokens, 1)?;

        Ok(())
    }
}
