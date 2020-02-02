use super::Diagnostic;
use crate::util::path_eq;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{quote, quote_spanned, ToTokens};
use std::collections::{HashMap, HashSet};
use syn::{
    parse_quote, spanned::Spanned, Attribute, AttributeArgs, Ident, Index, Item, Lit, Meta,
    NestedMeta, Signature,
};

fn meta_to_vec(meta: Meta) -> Result<Vec<NestedMeta>, Meta> {
    match meta {
        Meta::Path(_) => Ok(Vec::new()),
        Meta::List(list) => Ok(list.nested.into_iter().collect()),
        Meta::NameValue(_) => Err(meta),
    }
}

#[derive(Default)]
struct Class {
    items: HashSet<ClassItem>,
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

struct ClassItemMeta<'a> {
    sig: &'a Signature,
    parent_type: &'static str,
    meta: HashMap<String, Option<Lit>>,
}

impl<'a> ClassItemMeta<'a> {
    const METHOD_NAMES: &'static [&'static str] = &["name", "magic"];
    const PROPERTY_NAMES: &'static [&'static str] = &["name", "magic", "setter"];

    fn from_nested_meta(
        parent_type: &'static str,
        sig: &'a Signature,
        nested_meta: &[NestedMeta],
        names: &[&'static str],
    ) -> Result<Self, Diagnostic> {
        let mut extracted = Self {
            sig,
            parent_type,
            meta: HashMap::new(),
        };

        let validate_name = |name: &str, extracted: &Self| -> Result<(), Diagnostic> {
            if names.contains(&name) {
                if extracted.meta.contains_key(name) {
                    bail_span!(
                        &sig.ident,
                        "#[{}] must have only one '{}'",
                        parent_type,
                        name
                    );
                } else {
                    Ok(())
                }
            } else {
                bail_span!(
                    &sig.ident,
                    "#[{}({})] is not one of allowed attributes {}",
                    parent_type,
                    name,
                    names.join(", ")
                );
            }
        };

        for meta in nested_meta {
            let meta = match meta {
                NestedMeta::Meta(meta) => meta,
                NestedMeta::Lit(_) => continue,
            };

            match meta {
                Meta::NameValue(name_value) => {
                    if let Some(ident) = name_value.path.get_ident() {
                        let name = ident.to_string();
                        validate_name(&name, &extracted)?;
                        extracted.meta.insert(name, Some(name_value.lit.clone()));
                    }
                }
                Meta::Path(path) => {
                    if let Some(ident) = path.get_ident() {
                        let name = ident.to_string();
                        validate_name(&name, &extracted)?;
                        extracted.meta.insert(name, None);
                    } else {
                        continue;
                    }
                }
                _ => (),
            }
        }

        Ok(extracted)
    }

    fn _str(&self, key: &str) -> Result<Option<String>, Diagnostic> {
        Ok(match self.meta.get(key) {
            Some(Some(lit)) => {
                if let Lit::Str(s) = lit {
                    Some(s.value())
                } else {
                    bail_span!(
                        &self.sig.ident,
                        "#[{}({} = ...)] must be a string",
                        self.parent_type,
                        key
                    );
                }
            }
            Some(None) => {
                bail_span!(
                    &self.sig.ident,
                    "#[{}({} = ...)] is expected",
                    self.parent_type,
                    key,
                );
            }
            None => None,
        })
    }

    fn _bool(&self, key: &str) -> Result<bool, Diagnostic> {
        Ok(match self.meta.get(key) {
            Some(Some(_)) => {
                bail_span!(
                    &self.sig.ident,
                    "#[{}({})] is expected",
                    self.parent_type,
                    key,
                );
            }
            Some(None) => true,
            None => false,
        })
    }

    fn method_name(&self) -> Result<String, Diagnostic> {
        let name = self._str("name")?;
        let magic = self._bool("magic")?;
        Ok(if let Some(name) = name {
            name
        } else {
            let name = self.sig.ident.to_string();
            if magic {
                format!("__{}__", name)
            } else {
                name
            }
        })
    }

    fn setter(&self) -> Result<bool, Diagnostic> {
        self._bool("setter")
    }

    fn property_name(&self) -> Result<String, Diagnostic> {
        let magic = self._bool("magic")?;
        let setter = self._bool("setter")?;
        let name = self._str("name")?;

        Ok(if let Some(name) = name {
            name
        } else {
            let sig_name = self.sig.ident.to_string();
            let name = if setter {
                if let Some(name) = strip_prefix(&sig_name, "set_") {
                    if name.is_empty() {
                        bail_span!(
                            &self.sig.ident,
                            "A #[{}(setter)] fn with a set_* name must \
                             have something after \"set_\"",
                            self.parent_type
                        )
                    }
                    name.to_string()
                } else {
                    bail_span!(
                        &self.sig.ident,
                        "A #[{}(setter)] fn must either have a `name` \
                         parameter or a fn name along the lines of \"set_*\"",
                        self.parent_type
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
}

impl Class {
    fn add_item(&mut self, item: ClassItem, span: Span) -> Result<(), Diagnostic> {
        if self.items.insert(item) {
            Ok(())
        } else {
            Err(Diagnostic::span_error(
                span,
                "Duplicate #[py*] attribute on pyimpl".to_owned(),
            ))
        }
    }

    fn extract_method(sig: &Signature, meta: Meta) -> Result<ClassItem, Diagnostic> {
        let nesteds = meta_to_vec(meta).map_err(|meta| {
            err_span!(
                meta,
                "#[pymethod = \"...\"] cannot be a name/value, you probably meant \
                 #[pymethod(name = \"...\")]",
            )
        })?;

        let item_meta = ClassItemMeta::from_nested_meta(
            "pymethod",
            sig,
            &nesteds,
            ClassItemMeta::METHOD_NAMES,
        )?;
        Ok(ClassItem::Method {
            item_ident: sig.ident.clone(),
            py_name: item_meta.method_name()?,
        })
    }

    fn extract_classmethod(sig: &Signature, meta: Meta) -> Result<ClassItem, Diagnostic> {
        let nesteds = meta_to_vec(meta).map_err(|meta| {
            err_span!(
                meta,
                "#[pyclassmethod = \"...\"] cannot be a name/value, you probably meant \
                 #[pyclassmethod(name = \"...\")]",
            )
        })?;
        let item_meta = ClassItemMeta::from_nested_meta(
            "pyclassmethod",
            sig,
            &nesteds,
            ClassItemMeta::METHOD_NAMES,
        )?;
        Ok(ClassItem::ClassMethod {
            item_ident: sig.ident.clone(),
            py_name: item_meta.method_name()?,
        })
    }

    fn extract_property(sig: &Signature, meta: Meta) -> Result<ClassItem, Diagnostic> {
        let nesteds = meta_to_vec(meta).map_err(|meta| {
            err_span!(
                meta,
                "#[pyproperty = \"...\"] cannot be a name/value, you probably meant \
                 #[pyproperty(name = \"...\")]"
            )
        })?;
        let item_meta = ClassItemMeta::from_nested_meta(
            "pyproperty",
            sig,
            &nesteds,
            ClassItemMeta::PROPERTY_NAMES,
        )?;
        Ok(ClassItem::Property {
            py_name: item_meta.property_name()?,
            item_ident: sig.ident.clone(),
            setter: item_meta.setter()?,
        })
    }

    fn extract_slot(sig: &Signature, meta: Meta) -> Result<ClassItem, Diagnostic> {
        let pyslot_err = "#[pyslot] must be of the form #[pyslot] or #[pyslot(slotname)]";
        let nesteds = meta_to_vec(meta).map_err(|meta| err_span!(meta, "{}", pyslot_err))?;
        if nesteds.len() > 1 {
            return Err(Diagnostic::spanned_error(&quote!(#(#nesteds)*), pyslot_err));
        }
        let slot_ident = if nesteds.is_empty() {
            let ident_str = sig.ident.to_string();
            if let Some(stripped) = strip_prefix(&ident_str, "tp_") {
                proc_macro2::Ident::new(stripped, sig.ident.span())
            } else {
                sig.ident.clone()
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
            item_ident: sig.ident.clone(),
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
            if name == "pymethod" {
                self.add_item(Self::extract_method(sig, meta)?, meta_span)?;
            } else if name == "pyclassmethod" {
                self.add_item(Self::extract_classmethod(sig, meta)?, meta_span)?;
            } else if name == "pyproperty" {
                self.add_item(Self::extract_property(sig, meta)?, meta_span)?;
            } else if name == "pyslot" {
                self.add_item(Self::extract_slot(sig, meta)?, meta_span)?;
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

fn extract_impl_items(mut items: Vec<ItemSig>) -> Result<TokenStream2, Diagnostic> {
    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    let mut class = Class::default();

    for item in items.iter_mut() {
        push_diag_result!(
            diagnostics,
            class.extract_item_from_syn(&mut item.attrs, item.sig),
        );
    }

    let mut properties: HashMap<&str, (Option<&Ident>, Option<&Ident>)> = HashMap::new();
    for item in class.items.iter() {
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
    let methods = class.items.into_iter().filter_map(|item| match item {
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
            let into_func = quote_spanned! {item_ident.span()=>
                ::rustpython_vm::function::IntoPyNativeFunc::into_func(Self::#item_ident)
            };
            Some(quote! {
                (*class.slots.borrow_mut()).#slot_ident = Some(#into_func);
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
                        Some(ItemSig { attrs, sig })
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
                        Some(ItemSig { attrs, sig })
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
    attr_name: &'static str,
    attr: AttributeArgs,
    attrs: &[Attribute],
) -> Result<TokenStream2, Diagnostic> {
    let mut class_name = None;
    for attr in attr {
        if let NestedMeta::Meta(meta) = attr {
            if let Meta::NameValue(name_value) = meta {
                if path_eq(&name_value.path, "name") {
                    if let Lit::Str(s) = name_value.lit {
                        class_name = Some(s.value());
                    } else {
                        bail_span!(
                            name_value.lit,
                            "#[{}(name = ...)] must be a string",
                            attr_name
                        );
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
            const NAME: &'static str = #class_name;
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

    let class_def = generate_class_def(&ident, "pyclass", attr, &attrs)?;

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
    let class_def = generate_class_def(&struc.ident, "pystruct_sequence", attr, &struc.attrs)?;
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
                    ctx.new_property(
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
                    )?),*],
                );
                ::rustpython_vm::pyobject::PyValue::into_ref_with_type(tuple, vm, cls)
            }
        }
        impl ::rustpython_vm::pyobject::PyClassImpl for #ty {
            fn impl_extend_class(
                ctx: &::rustpython_vm::pyobject::PyContext,
                class: &::rustpython_vm::obj::objtype::PyClassRef,
            ) {
                #(#properties)*
            }

            fn make_class(
                ctx: &::rustpython_vm::pyobject::PyContext
            ) -> ::rustpython_vm::obj::objtype::PyClassRef {
                let py_class = ctx.new_class(<Self as ::rustpython_vm::pyobject::PyClassDef>::NAME, ctx.tuple_type());
                Self::extend_class(ctx, &py_class);
                py_class
            }
        }
    };
    Ok(ret)
}

fn strip_prefix<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    if s.starts_with(prefix) {
        Some(&s[prefix.len()..])
    } else {
        None
    }
}
