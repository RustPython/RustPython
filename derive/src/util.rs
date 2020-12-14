use indexmap::map::IndexMap;
use proc_macro2::{Span, TokenStream};
use quote::{quote, ToTokens};
use std::collections::HashMap;
use syn::{spanned::Spanned, Attribute, Ident, Meta, MetaList, NestedMeta, Path, Result, UseTree};
use syn_ext::ext::{AttributeExt as SynAttributeExt, *};
use syn_ext::types::PunctuatedNestedMeta;

pub(crate) const ALL_ALLOWED_NAMES: &[&str] = &[
    "pymethod",
    "pyclassmethod",
    "pyproperty",
    "pyfunction",
    "pyclass",
    "pystruct_sequence",
    "pyattr",
    "pyslot",
    "extend_class",
];

#[derive(Default)]
pub(crate) struct ItemNursery(IndexMap<(String, Vec<Attribute>), TokenStream>);

impl ItemNursery {
    pub fn add_item(
        &mut self,
        name: String,
        cfgs: Vec<Attribute>,
        tokens: TokenStream,
    ) -> Result<()> {
        if let Some(existing) = self.0.insert((name.clone(), cfgs), tokens) {
            Err(syn::Error::new_spanned(
                existing,
                format!("Duplicated #[py*] attribute found for '{}'", name),
            ))
        } else {
            Ok(())
        }
    }
}

impl ToTokens for ItemNursery {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        tokens.extend(self.0.iter().map(|((_, cfgs), item)| {
            quote! {
                #( #cfgs )*
                {
                    #item
                }
            }
        }))
    }
}

#[derive(Clone)]
pub(crate) struct ContentItemInner {
    pub index: usize,
    pub attr_name: String,
}

pub(crate) trait ContentItem {
    fn inner(&self) -> &ContentItemInner;
    fn index(&self) -> usize {
        self.inner().index
    }
    fn attr_name(&self) -> &str {
        self.inner().attr_name.as_str()
    }
    fn new_syn_error(&self, span: Span, message: &str) -> syn::Error {
        syn::Error::new(span, format!("#[{}] {}", self.attr_name(), message))
    }
}

pub(crate) struct ItemMetaInner {
    pub item_ident: Ident,
    pub meta_ident: Ident,
    pub meta_map: HashMap<String, (usize, Meta)>,
}

impl ItemMetaInner {
    pub fn from_nested<I>(
        item_ident: Ident,
        meta_ident: Ident,
        nested: I,
        allowed_names: &[&'static str],
    ) -> Result<Self>
    where
        I: std::iter::Iterator<Item = NestedMeta>,
    {
        let (meta_map, lits) = nested.into_unique_map_and_lits(|path| {
            if let Some(ident) = path.get_ident() {
                let name = ident.to_string();
                if allowed_names.contains(&name.as_str()) {
                    Ok(Some(name))
                } else {
                    Err(syn::Error::new_spanned(
                        ident,
                        format!(
                            "#[{}({})] is not one of allowed attributes {}",
                            meta_ident.to_string(),
                            name,
                            allowed_names.join(", ")
                        ),
                    ))
                }
            } else {
                Ok(None)
            }
        })?;
        if !lits.is_empty() {
            return Err(syn::Error::new_spanned(
                &meta_ident,
                format!("#[{}(..)] cannot contain literal", meta_ident.to_string()),
            ));
        }

        Ok(Self {
            item_ident,
            meta_ident,
            meta_map,
        })
    }

    pub fn item_name(&self) -> String {
        self.item_ident.to_string()
    }

    pub fn meta_name(&self) -> String {
        self.meta_ident.to_string()
    }

    pub fn _optional_str(&self, key: &str) -> Result<Option<String>> {
        let value = if let Some((_, meta)) = self.meta_map.get(key) {
            match meta {
                Meta::NameValue(syn::MetaNameValue {
                    lit: syn::Lit::Str(lit),
                    ..
                }) => Some(lit.value()),
                other => {
                    return Err(syn::Error::new_spanned(
                        other,
                        format!(
                            "#[{}({} = ...)] must exist as a string",
                            self.meta_name(),
                            key
                        ),
                    ));
                }
            }
        } else {
            None
        };
        Ok(value)
    }

    pub fn _bool(&self, key: &str) -> Result<bool> {
        let value = if let Some((_, meta)) = self.meta_map.get(key) {
            match meta {
                Meta::NameValue(syn::MetaNameValue {
                    lit: syn::Lit::Bool(lit),
                    ..
                }) => lit.value,
                Meta::Path(_) => true,
                other => {
                    return Err(syn::Error::new_spanned(
                        other,
                        format!("#[{}({})] is expected", self.meta_name(), key),
                    ))
                }
            }
        } else {
            false
        };
        Ok(value)
    }
}

pub(crate) trait ItemMeta: Sized {
    const ALLOWED_NAMES: &'static [&'static str];

    fn from_attr(item_ident: Ident, attr: &Attribute) -> Result<Self> {
        let (meta_ident, nested) = attr.ident_and_promoted_nested()?;
        Self::from_nested(item_ident, meta_ident.clone(), nested.into_iter())
    }

    fn from_nested<I>(item_ident: Ident, meta_ident: Ident, nested: I) -> Result<Self>
    where
        I: std::iter::Iterator<Item = NestedMeta>,
    {
        Ok(Self::from_inner(ItemMetaInner::from_nested(
            item_ident,
            meta_ident,
            nested,
            Self::ALLOWED_NAMES,
        )?))
    }

    fn from_inner(inner: ItemMetaInner) -> Self;
    fn inner(&self) -> &ItemMetaInner;

    fn simple_name(&self) -> Result<String> {
        let inner = self.inner();
        Ok(inner
            ._optional_str("name")?
            .unwrap_or_else(|| inner.item_name()))
    }

    fn optional_name(&self) -> Option<String> {
        self.inner()._optional_str("name").ok().flatten()
    }

    fn new_meta_error(&self, msg: &str) -> syn::Error {
        let inner = self.inner();
        syn::Error::new_spanned(
            &inner.meta_ident,
            format!("#[{}] {}", inner.meta_name(), msg),
        )
    }
}
pub(crate) struct SimpleItemMeta(pub ItemMetaInner);

impl ItemMeta for SimpleItemMeta {
    const ALLOWED_NAMES: &'static [&'static str] = &["name"];

    fn from_inner(inner: ItemMetaInner) -> Self {
        Self(inner)
    }
    fn inner(&self) -> &ItemMetaInner {
        &self.0
    }
}

pub(crate) struct ClassItemMeta(ItemMetaInner);

impl ItemMeta for ClassItemMeta {
    const ALLOWED_NAMES: &'static [&'static str] = &["module", "name", "base"];

    fn from_inner(inner: ItemMetaInner) -> Self {
        Self(inner)
    }
    fn inner(&self) -> &ItemMetaInner {
        &self.0
    }
}

impl ClassItemMeta {
    pub fn class_name(&self) -> Result<String> {
        const KEY: &str = "name";
        let inner = self.inner();
        let value = if let Some((_, meta)) = inner.meta_map.get(KEY) {
            match meta {
                Meta::NameValue(syn::MetaNameValue {
                    lit: syn::Lit::Str(lit),
                    ..
                }) => Some(lit.value()),
                Meta::Path(_) => Some(inner.item_name()),
                _ => None,
            }
        } else {
            None
        }.ok_or_else(|| syn::Error::new_spanned(
            &inner.meta_ident,
            format!(
                "#[{attr_name}(name = ...)] must exist as a string. Try #[{attr_name}(name)] to use rust type name.",
                attr_name=inner.meta_name()
            ),
        ))?;
        Ok(value)
    }

    pub fn base(&self) -> Result<Option<String>> {
        self.inner()._optional_str("base")
    }

    pub fn module(&self) -> Result<Option<String>> {
        const KEY: &str = "module";
        let inner = self.inner();
        let value = if let Some((_, meta)) = inner.meta_map.get(KEY) {
            match meta {
                Meta::NameValue(syn::MetaNameValue {
                    lit: syn::Lit::Str(lit),
                    ..
                }) => Ok(Some(lit.value())),
                Meta::NameValue(syn::MetaNameValue {
                    lit: syn::Lit::Bool(lit),
                    ..
                }) => if lit.value {
                    Err(lit.span())
                } else {
                    Ok(None)
                }
                other => Err(other.span()),
            }
        } else {
            Err(inner.item_ident.span())
        }.map_err(|span| syn::Error::new(
            span,
            format!(
                "#[{attr_name}(module = ...)] must exist as a string or false. Try #[{attr_name}(module=false)] for built-in types.",
                attr_name=inner.meta_name()
            ),
        ))?;
        Ok(value)
    }

    // pub fn mandatory_module(&self) -> Result<String> {
    //     let inner = self.inner();
    //     let value = self.module().ok().flatten().
    //     ok_or_else(|| syn::Error::new_spanned(
    //         &inner.meta_ident,
    //         format!(
    //             "#[{attr_name}(module = ...)] must exist as a string. Built-in module is not allowed here.",
    //             attr_name=inner.meta_name()
    //         ),
    //     ))?;
    //     Ok(value)
    // }
}

pub(crate) fn path_eq(path: &Path, s: &str) -> bool {
    path.get_ident().map_or(false, |id| id == s)
}

pub(crate) trait AttributeExt: SynAttributeExt {
    fn promoted_nested(&self) -> Result<PunctuatedNestedMeta>;
    fn ident_and_promoted_nested(&self) -> Result<(&Ident, PunctuatedNestedMeta)>;
    fn try_remove_name(&mut self, name: &str) -> Result<Option<syn::NestedMeta>>;
    fn fill_nested_meta<F>(&mut self, name: &str, new_item: F) -> Result<()>
    where
        F: Fn() -> NestedMeta;
}

impl AttributeExt for Attribute {
    fn promoted_nested(&self) -> Result<PunctuatedNestedMeta> {
        let list = self.promoted_list().map_err(|mut e| {
            let name = self.get_ident().unwrap().to_string();
            e.combine(syn::Error::new_spanned(
                self,
                format!(
                    "#[{name} = \"...\"] cannot be a name/value, you probably meant \
                     #[{name}(name = \"...\")]",
                    name = name,
                ),
            ));
            e
        })?;
        Ok(list.nested)
    }
    fn ident_and_promoted_nested(&self) -> Result<(&Ident, PunctuatedNestedMeta)> {
        Ok((self.get_ident().unwrap(), self.promoted_nested()?))
    }

    fn try_remove_name(&mut self, item_name: &str) -> Result<Option<syn::NestedMeta>> {
        self.try_meta_mut(|meta| {
            let nested = match meta {
                Meta::List(MetaList { ref mut nested, .. }) => Ok(nested),
                other => Err(syn::Error::new(
                    other.span(),
                    format!(
                        "#[{name}(...)] doesn't contain '{item}' to remove",
                        name = other.get_ident().unwrap().to_string(),
                        item = item_name
                    ),
                )),
            }?;

            let mut found = None;
            for (i, item) in nested.iter().enumerate() {
                let ident = if let Some(ident) = item.get_ident() {
                    ident
                } else {
                    continue;
                };
                if *ident != item_name {
                    continue;
                }
                if found.is_some() {
                    return Err(syn::Error::new(
                        item.span(),
                        format!(
                            "#[py..({}...)] must be unique but found multiple times",
                            item_name,
                        ),
                    ));
                }
                found = Some(i);
            }

            Ok(found.map(|idx| nested.remove(idx).into_value()))
        })
    }

    fn fill_nested_meta<F>(&mut self, name: &str, new_item: F) -> Result<()>
    where
        F: Fn() -> NestedMeta,
    {
        self.try_meta_mut(|meta| {
            let list = meta.promote_to_list(Default::default())?;
            let has_name = list
                .nested
                .iter()
                .any(|nmeta| nmeta.get_path().map_or(false, |p| path_eq(p, name)));
            if !has_name {
                list.nested.push(new_item())
            }
            Ok(())
        })
    }
}

pub(crate) fn pyclass_ident_and_attrs(item: &syn::Item) -> Result<(&Ident, &[Attribute])> {
    use syn::Item::*;
    match item {
        Struct(syn::ItemStruct { ident, attrs, .. }) => Ok((ident, attrs)),
        Enum(syn::ItemEnum { ident, attrs, .. }) => Ok((ident, attrs)),
        other => Err(syn::Error::new_spanned(
            other,
            "#[pyclass] can only be on a struct or enum declaration",
        )),
    }
}

pub(crate) trait ErrorVec: Sized {
    fn into_error(self) -> Option<syn::Error>;
    fn into_result(self) -> Result<()> {
        if let Some(error) = self.into_error() {
            Err(error)
        } else {
            Ok(())
        }
    }
    fn ok_or_push<T>(&mut self, r: Result<T>) -> Option<T>;
}

impl ErrorVec for Vec<syn::Error> {
    fn into_error(self) -> Option<syn::Error> {
        let mut iter = self.into_iter();
        if let Some(mut first) = iter.next() {
            for err in iter {
                first.combine(err);
            }
            Some(first)
        } else {
            None
        }
    }
    fn ok_or_push<T>(&mut self, r: Result<T>) -> Option<T> {
        match r {
            Ok(v) => Some(v),
            Err(e) => {
                self.push(e);
                None
            }
        }
    }
}

macro_rules! iter_chain {
    ($($it:expr),*$(,)?) => {
        ::std::iter::empty()
            $(.chain(::std::iter::once($it)))*
    };
}

pub(crate) fn iter_use_idents<F>(item_use: &syn::ItemUse, mut f: F) -> Result<()>
where
    F: FnMut(&syn::Ident, bool) -> Result<()>,
{
    match &item_use.tree {
        UseTree::Name(name) => f(&name.ident, true)?,
        UseTree::Rename(rename) => f(&rename.rename, true)?,
        UseTree::Path(path) => match &*path.tree {
            UseTree::Name(name) => f(&name.ident, true)?,
            UseTree::Rename(rename) => f(&rename.rename, true)?,
            other => iter_use_tree_idents(other, &mut f)?,
        },
        other => iter_use_tree_idents(other, &mut f)?,
    }
    Ok(())
}

fn iter_use_tree_idents<F>(tree: &syn::UseTree, f: &mut F) -> Result<()>
where
    F: FnMut(&syn::Ident, bool) -> Result<()>,
{
    match tree {
        UseTree::Name(name) => f(&name.ident, false)?,
        UseTree::Rename(rename) => f(&rename.rename, false)?,
        UseTree::Path(path) => iter_use_tree_idents(&*path.tree, f)?,
        UseTree::Group(syn::UseGroup { items, .. }) => {
            for subtree in items {
                iter_use_tree_idents(subtree, f)?;
            }
        }
        UseTree::Glob(glob) => {
            return Err(syn::Error::new_spanned(glob, "#[py*] doesn't allow '*'"))
        }
    }
    Ok(())
}
