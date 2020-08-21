use proc_macro2::{Span, TokenStream};
use quote::{quote, ToTokens};
use std::collections::HashMap;
use syn::{spanned::Spanned, Attribute, Ident, Meta, MetaList, NestedMeta, Path, Result};
use syn_ext::ext::{AttributeExt as SynAttributeExt, *};
use syn_ext::types::PunctuatedNestedMeta;

pub(crate) const ALL_ALLOWED_NAMES: &[&str] = &[
    "pymethod",
    "pyproperty",
    "pyfunction",
    "pyclass",
    "pystruct_sequence",
    "pyattr",
];

#[derive(Default)]
pub(crate) struct ItemNursery(HashMap<(String, Vec<Attribute>), TokenStream>);

impl ItemNursery {
    pub fn add_item(
        &mut self,
        name: String,
        cfgs: Vec<Attribute>,
        tokens: TokenStream,
    ) -> Result<()> {
        if let Some(existing) = self.0.insert((name, cfgs), tokens) {
            Err(syn::Error::new_spanned(
                existing,
                "Duplicated #[py*] attribute found",
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
    pub ident: Ident,
    pub parent_type: &'static str,
    pub meta: HashMap<String, (usize, Meta)>,
}

impl ItemMetaInner {
    pub fn from_nested<I>(
        parent_type: &'static str,
        ident: &Ident,
        nested: I,
        allowed_names: &[&'static str],
    ) -> Result<Self>
    where
        I: std::iter::Iterator<Item = NestedMeta>,
    {
        let (named_map, lits) = nested.into_unique_map_and_lits(|path| {
            if let Some(ident) = path.get_ident() {
                let name = ident.to_string();
                if allowed_names.contains(&name.as_str()) {
                    Ok(Some(name))
                } else {
                    Err(syn::Error::new_spanned(
                        ident,
                        format!(
                            "#[{}({})] is not one of allowed attributes {}",
                            parent_type,
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
                ident,
                format!("#[{}(..)] cannot contain literal", parent_type),
            ));
        }

        Ok(Self {
            ident: ident.clone(),
            parent_type,
            meta: named_map,
        })
    }

    pub fn _optional_str(&self, key: &str) -> Result<Option<String>> {
        let value = if let Some((_, meta)) = self.meta.get(key) {
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
                            self.parent_type, key
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
        let value = if let Some((_, meta)) = self.meta.get(key) {
            match meta {
                Meta::NameValue(syn::MetaNameValue {
                    lit: syn::Lit::Bool(lit),
                    ..
                }) => lit.value,
                Meta::Path(_) => true,
                other => {
                    return Err(syn::Error::new_spanned(
                        other,
                        format!("#[{}({})] is expected", self.parent_type, key),
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

    fn from_nested<I>(parent_type: &'static str, ident: &Ident, nested: I) -> Result<Self>
    where
        I: std::iter::Iterator<Item = NestedMeta>,
    {
        Ok(Self::from_inner(ItemMetaInner::from_nested(
            parent_type,
            ident,
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
            .unwrap_or_else(|| inner.ident.to_string()))
    }

    fn optional_name(&self) -> Option<String> {
        self.inner()._optional_str("name").ok().flatten()
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
    const ALLOWED_NAMES: &'static [&'static str] = &["module", "name"];

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
        let value = if let Some((_, meta)) = inner.meta.get(KEY) {
            match meta {
                Meta::NameValue(syn::MetaNameValue {
                    lit: syn::Lit::Str(lit),
                    ..
                }) => Some(lit.value()),
                Meta::Path(_) => Some(inner.ident.to_string()),
                _ => None,
            }
        } else {
            None
        }.ok_or_else(|| syn::Error::new_spanned(
            &inner.ident,
            format!(
                "#[{attr_name}(name = ...)] must exist as a string. Try #[{attr_name}(name)] to use rust type name.",
                attr_name=inner.parent_type
            ),
        ))?;
        Ok(value)
    }

    pub fn module(&self) -> Result<Option<String>> {
        self.inner()._optional_str("module")
    }
}

pub(crate) fn path_eq(path: &Path, s: &str) -> bool {
    path.get_ident().map_or(false, |id| id == s)
}

pub(crate) trait AttributeExt: SynAttributeExt {
    fn promoted_nested(&self) -> Result<PunctuatedNestedMeta>;
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

pub(crate) struct ItemIdent<'a> {
    pub attrs: &'a mut Vec<Attribute>,
    pub ident: &'a Ident,
}
