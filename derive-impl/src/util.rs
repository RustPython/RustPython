use itertools::Itertools;
use proc_macro2::{Span, TokenStream};
use quote::{quote, ToTokens};
use std::collections::{HashMap, HashSet};
use syn::{
    spanned::Spanned, Attribute, Ident, Meta, MetaList, NestedMeta, Result, Signature, UseTree,
};
use syn_ext::{
    ext::{AttributeExt as SynAttributeExt, *},
    types::PunctuatedNestedMeta,
};

pub(crate) const ALL_ALLOWED_NAMES: &[&str] = &[
    "pymethod",
    "pyclassmethod",
    "pystaticmethod",
    "pygetset",
    "pyfunction",
    "pyclass",
    "pyexception",
    "pystruct_sequence",
    "pyattr",
    "pyslot",
    "extend_class",
    "pymember",
];

#[derive(Clone)]
struct NurseryItem {
    attr_name: Ident,
    py_names: Vec<String>,
    cfgs: Vec<Attribute>,
    tokens: TokenStream,
    sort_order: usize,
}

#[derive(Default)]
pub(crate) struct ItemNursery(Vec<NurseryItem>);

pub(crate) struct ValidatedItemNursery(ItemNursery);

impl ItemNursery {
    pub fn add_item(
        &mut self,
        attr_name: Ident,
        py_names: Vec<String>,
        cfgs: Vec<Attribute>,
        tokens: TokenStream,
        sort_order: usize,
    ) -> Result<()> {
        self.0.push(NurseryItem {
            attr_name,
            py_names,
            cfgs,
            tokens,
            sort_order,
        });
        Ok(())
    }

    pub fn validate(self) -> Result<ValidatedItemNursery> {
        let mut by_name: HashSet<(String, Vec<Attribute>)> = HashSet::new();
        for item in &self.0 {
            for py_name in &item.py_names {
                let inserted = by_name.insert((py_name.clone(), item.cfgs.clone()));
                if !inserted {
                    return Err(syn::Error::new(
                        item.attr_name.span(),
                        format!("Duplicated #[py*] attribute found for {:?}", &item.py_names),
                    ));
                }
            }
        }
        Ok(ValidatedItemNursery(self))
    }
}

impl ToTokens for ValidatedItemNursery {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let mut sorted = self.0 .0.clone();
        sorted.sort_by(|a, b| a.sort_order.cmp(&b.sort_order));
        tokens.extend(sorted.iter().map(|item| {
            let cfgs = &item.cfgs;
            let tokens = &item.tokens;
            quote! {
                #( #cfgs )*
                {
                    #tokens
                }
            }
        }))
    }
}

#[derive(Clone)]
pub(crate) struct ContentItemInner<T> {
    pub index: usize,
    pub attr_name: T,
}

pub(crate) trait ContentItem {
    type AttrName: std::str::FromStr + std::fmt::Display;

    fn inner(&self) -> &ContentItemInner<Self::AttrName>;
    fn index(&self) -> usize {
        self.inner().index
    }
    fn attr_name(&self) -> &Self::AttrName {
        &self.inner().attr_name
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
                    Err(err_span!(
                        ident,
                        "#[{meta_ident}({name})] is not one of allowed attributes [{}]",
                        allowed_names.iter().format(", ")
                    ))
                }
            } else {
                Ok(None)
            }
        })?;
        if !lits.is_empty() {
            bail_span!(meta_ident, "#[{meta_ident}(..)] cannot contain literal")
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
            let Meta::NameValue(syn::MetaNameValue {
                lit: syn::Lit::Str(lit), ..
            }) = meta else {
                bail_span!(meta, "#[{}({} = ...)] must exist as a string", self.meta_name(), key)
            };
            Some(lit.value())
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
                _ => bail_span!(meta, "#[{}({})] is expected", self.meta_name(), key),
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
        err_span!(inner.meta_ident, "#[{}] {}", inner.meta_name(), msg)
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

pub(crate) struct AttrItemMeta(pub ItemMetaInner);

impl ItemMeta for AttrItemMeta {
    const ALLOWED_NAMES: &'static [&'static str] = &["name", "once"];

    fn from_inner(inner: ItemMetaInner) -> Self {
        Self(inner)
    }
    fn inner(&self) -> &ItemMetaInner {
        &self.0
    }
}

pub(crate) struct ClassItemMeta(ItemMetaInner);

impl ItemMeta for ClassItemMeta {
    const ALLOWED_NAMES: &'static [&'static str] = &["module", "name", "base", "metaclass"];

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
        if let Some((_, meta)) = inner.meta_map.get(KEY) {
            match meta {
                Meta::NameValue(syn::MetaNameValue {
                    lit: syn::Lit::Str(lit),
                    ..
                }) => return Ok(lit.value()),
                Meta::Path(_) => return Ok(inner.item_name()),
                _ => {}
            }
        }
        bail_span!(
            inner.meta_ident,
            "#[{attr_name}(name = ...)] must exist as a string. Try \
             #[{attr_name}(name)] to use rust type name.",
            attr_name = inner.meta_name()
        )
    }

    pub fn base(&self) -> Result<Option<String>> {
        self.inner()._optional_str("base")
    }

    pub fn metaclass(&self) -> Result<Option<String>> {
        self.inner()._optional_str("metaclass")
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
                "#[{attr_name}(module = ...)] must exist as a string or false. Try #[{attr_name}(module = false)] for built-in types.",
                attr_name=inner.meta_name()
            ),
        ))?;
        Ok(value)
    }

    // pub fn mandatory_module(&self) -> Result<String> {
    //     let inner = self.inner();
    //     let value = self.module().ok().flatten().
    //     ok_or_else(|| err_span!(
    //         inner.meta_ident,
    //         "#[{attr_name}(module = ...)] must exist as a string. Built-in module is not allowed here.",
    //         attr_name = inner.meta_name()
    //     ))?;
    //     Ok(value)
    // }
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
            e.combine(err_span!(
                self,
                "#[{name} = \"...\"] cannot be a name/value, you probably meant \
                 #[{name}(name = \"...\")]",
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
                        name = other.get_ident().unwrap(),
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
                .any(|nmeta| nmeta.get_path().map_or(false, |p| p.is_ident(name)));
            if !has_name {
                list.nested.push(new_item())
            }
            Ok(())
        })
    }
}

pub(crate) fn pyclass_ident_and_attrs(item: &syn::Item) -> Result<(&Ident, &[Attribute])> {
    use syn::Item::*;
    Ok(match item {
        Struct(syn::ItemStruct { ident, attrs, .. }) => (ident, attrs),
        Enum(syn::ItemEnum { ident, attrs, .. }) => (ident, attrs),
        Use(item_use) => (
            iter_use_idents(item_use, |ident, _is_unique| Ok(ident))?
                .into_iter()
                .exactly_one()
                .map_err(|_| {
                    err_span!(
                        item_use,
                        "#[pyclass] can only be on single name use statement",
                    )
                })?,
            &item_use.attrs,
        ),
        other => {
            bail_span!(
                other,
                "#[pyclass] can only be on a struct, enum or use declaration",
            )
        }
    })
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

pub(crate) fn iter_use_idents<'a, F, R: 'a>(item_use: &'a syn::ItemUse, mut f: F) -> Result<Vec<R>>
where
    F: FnMut(&'a syn::Ident, bool) -> Result<R>,
{
    let mut result = Vec::new();
    match &item_use.tree {
        UseTree::Name(name) => result.push(f(&name.ident, true)?),
        UseTree::Rename(rename) => result.push(f(&rename.rename, true)?),
        UseTree::Path(path) => match &*path.tree {
            UseTree::Name(name) => result.push(f(&name.ident, true)?),
            UseTree::Rename(rename) => result.push(f(&rename.rename, true)?),
            other => iter_use_tree_idents(other, &mut result, &mut f)?,
        },
        other => iter_use_tree_idents(other, &mut result, &mut f)?,
    }
    Ok(result)
}

fn iter_use_tree_idents<'a, F, R: 'a>(
    tree: &'a syn::UseTree,
    result: &mut Vec<R>,
    f: &mut F,
) -> Result<()>
where
    F: FnMut(&'a syn::Ident, bool) -> Result<R>,
{
    match tree {
        UseTree::Name(name) => result.push(f(&name.ident, false)?),
        UseTree::Rename(rename) => result.push(f(&rename.rename, false)?),
        UseTree::Path(path) => iter_use_tree_idents(&path.tree, result, f)?,
        UseTree::Group(syn::UseGroup { items, .. }) => {
            for subtree in items {
                iter_use_tree_idents(subtree, result, f)?;
            }
        }
        UseTree::Glob(glob) => {
            bail_span!(glob, "#[py*] doesn't allow '*'")
        }
    }
    Ok(())
}

// Best effort attempt to generate a template from which a
// __text_signature__ can be created.
pub(crate) fn text_signature(sig: &Signature, name: &str) -> String {
    let signature = func_sig(sig);
    if signature.starts_with("$self") {
        format!("{}({})", name, signature)
    } else {
        format!("{}({}, {})", name, "$module", signature)
    }
}

fn func_sig(sig: &Signature) -> String {
    sig.inputs
        .iter()
        .filter_map(|arg| {
            use syn::FnArg::*;
            let arg = match arg {
                Typed(typed) => typed,
                Receiver(_) => return Some("$self".to_owned()),
            };
            let ty = arg.ty.as_ref();
            let ty = quote!(#ty).to_string();
            if ty == "FuncArgs" {
                return Some("*args, **kwargs".to_owned());
            }
            if ty.starts_with('&') && ty.ends_with("VirtualMachine") {
                return None;
            }
            let ident = match arg.pat.as_ref() {
                syn::Pat::Ident(p) => p.ident.to_string(),
                // FIXME: other => unreachable!("function arg pattern must be ident but found `{}`", quote!(fn #ident(.. #other ..))),
                other => quote!(#other).to_string(),
            };
            if ident == "zelf" {
                return Some("$self".to_owned());
            }
            if ident == "vm" {
                unreachable!("type &VirtualMachine(`{}`) must be filtered already", ty);
            }
            Some(ident)
        })
        .collect::<Vec<_>>()
        .join(", ")
}
