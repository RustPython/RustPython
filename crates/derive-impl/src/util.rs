use itertools::Itertools;
use proc_macro2::{Span, TokenStream};
use quote::{ToTokens, quote};
use std::collections::{HashMap, HashSet};
use syn::visit::Visit;
use syn::visit_mut::VisitMut;
use syn::{
    Attribute, FnArg, Ident, Result, Signature, Type, UseTree, ext::IdentExt, spanned::Spanned,
};
use syn_ext::{
    ext::{AttributeExt as SynAttributeExt, *},
    types::*,
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
    pub(crate) fn add_item(
        &mut self,
        attr_name: Ident,
        py_names: Vec<String>,
        cfgs: Vec<Attribute>,
        tokens: TokenStream,
        sort_order: usize,
    ) {
        self.0.push(NurseryItem {
            attr_name,
            py_names,
            cfgs,
            tokens,
            sort_order,
        });
    }

    pub(crate) fn validate(self) -> Result<ValidatedItemNursery> {
        let mut by_name: HashSet<(String, Vec<Attribute>)> = HashSet::new();
        for item in &self.0 {
            for py_name in &item.py_names {
                let inserted = by_name.insert((py_name.clone(), item.cfgs.clone()));
                if !inserted {
                    return Err(syn::Error::new(
                        item.attr_name.span(),
                        format!("Duplicated #[py*] attribute found for {:?}", item.py_names),
                    ));
                }
            }
        }
        Ok(ValidatedItemNursery(self))
    }
}

impl ToTokens for ValidatedItemNursery {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let mut sorted = self.0.0.clone();
        sorted.sort_by_key(|a| a.sort_order);
        tokens.extend(sorted.iter().map(|item| {
            let cfgs = &item.cfgs;
            let tokens = &item.tokens;
            quote! {
                #(#cfgs)*
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
    type AttrName: core::str::FromStr + core::fmt::Display;

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
    pub(crate) fn from_nested<I>(
        item_ident: Ident,
        meta_ident: Ident,
        nested: I,
        allowed_names: &[&'static str],
    ) -> Result<Self>
    where
        I: core::iter::Iterator<Item = NestedMeta>,
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

    pub(crate) fn contains_key(&self, key: &str) -> bool {
        self.meta_map.contains_key(key)
    }

    pub(crate) fn item_name(&self) -> String {
        self.item_ident.to_string()
    }

    pub(crate) fn meta_name(&self) -> String {
        self.meta_ident.to_string()
    }

    pub(crate) fn _optional_str(&self, key: &str) -> Result<Option<String>> {
        let value = if let Some((_, meta)) = self.meta_map.get(key) {
            let Meta::NameValue(syn::MetaNameValue {
                value:
                    syn::Expr::Lit(syn::ExprLit {
                        lit: syn::Lit::Str(lit),
                        ..
                    }),
                ..
            }) = meta
            else {
                bail_span!(
                    meta,
                    "#[{}({} = ...)] must exist as a string",
                    self.meta_name(),
                    key
                )
            };
            Some(lit.value())
        } else {
            None
        };
        Ok(value)
    }

    pub(crate) fn _optional_path(&self, key: &str) -> Result<Option<syn::Path>> {
        let value = if let Some((_, meta)) = self.meta_map.get(key) {
            let Meta::NameValue(syn::MetaNameValue { value, .. }) = meta else {
                bail_span!(
                    meta,
                    "#[{}({} = ...)] must be a name-value pair",
                    self.meta_name(),
                    key
                )
            };

            // Try to parse as a Path (identifier or path like Foo or foo::Bar)
            match syn::parse2::<syn::Path>(value.to_token_stream()) {
                Ok(path) => Some(path),
                Err(_) => {
                    bail_span!(
                        value,
                        "#[{}({} = ...)] must be a valid type path (e.g., PyBaseException)",
                        self.meta_name(),
                        key
                    )
                }
            }
        } else {
            None
        };
        Ok(value)
    }

    pub(crate) fn _bool(&self, key: &str) -> Result<bool> {
        let value = if let Some((_, meta)) = self.meta_map.get(key) {
            match meta {
                Meta::NameValue(syn::MetaNameValue {
                    value:
                        syn::Expr::Lit(syn::ExprLit {
                            lit: syn::Lit::Bool(lit),
                            ..
                        }),
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

    pub(crate) fn _optional_list(
        &self,
        key: &str,
    ) -> Result<Option<impl core::iter::Iterator<Item = &'_ NestedMeta>>> {
        let value = if let Some((_, meta)) = self.meta_map.get(key) {
            let Meta::List(MetaList {
                path: _, nested, ..
            }) = meta
            else {
                bail_span!(meta, "#[{}({}(...))] must be a list", self.meta_name(), key)
            };
            Some(nested.into_iter())
        } else {
            None
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
        I: core::iter::Iterator<Item = NestedMeta>,
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

pub(crate) struct ModuleItemMeta(pub ItemMetaInner);

impl ItemMeta for ModuleItemMeta {
    const ALLOWED_NAMES: &'static [&'static str] = &["name", "sub"];

    fn from_inner(inner: ItemMetaInner) -> Self {
        Self(inner)
    }

    fn inner(&self) -> &ItemMetaInner {
        &self.0
    }
}

impl ModuleItemMeta {
    pub(crate) fn sub(&self) -> Result<bool> {
        self.inner()._bool("sub")
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
    const ALLOWED_NAMES: &'static [&'static str] = &[
        "module",
        "name",
        "base",
        "metaclass",
        "unhashable",
        "ctx",
        "impl",
        "traverse",
        "clear", // tp_clear
        "payload",
    ];

    fn from_inner(inner: ItemMetaInner) -> Self {
        Self(inner)
    }

    fn inner(&self) -> &ItemMetaInner {
        &self.0
    }
}

impl ClassItemMeta {
    pub(crate) fn class_name(&self) -> Result<String> {
        const KEY: &str = "name";
        let inner = self.inner();
        if let Some((_, meta)) = inner.meta_map.get(KEY) {
            match meta {
                Meta::NameValue(syn::MetaNameValue {
                    value:
                        syn::Expr::Lit(syn::ExprLit {
                            lit: syn::Lit::Str(lit),
                            ..
                        }),
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

    pub(crate) fn ctx_name(&self) -> Result<Option<String>> {
        self.inner()._optional_str("ctx")
    }

    pub(crate) fn manual_payload(&self) -> Result<bool> {
        Ok(self.inner()._optional_str("payload")?.as_deref() == Some("manual"))
    }

    pub(crate) fn base(&self) -> Result<Option<syn::Path>> {
        self.inner()._optional_path("base")
    }

    pub(crate) fn unhashable(&self) -> Result<bool> {
        self.inner()._bool("unhashable")
    }

    pub(crate) fn metaclass(&self) -> Result<Option<String>> {
        self.inner()._optional_str("metaclass")
    }

    pub(crate) fn module(&self) -> Result<Option<String>> {
        const KEY: &str = "module";
        let inner = self.inner();
        let value = if let Some((_, meta)) = inner.meta_map.get(KEY) {
            match meta {
                Meta::NameValue(syn::MetaNameValue {
                    value: syn::Expr::Lit(syn::ExprLit{lit:syn::Lit::Str(lit),..}),
                    ..
                }) => Ok(Some(lit.value())),
                Meta::NameValue(syn::MetaNameValue {
                    value: syn::Expr::Lit(syn::ExprLit{lit:syn::Lit::Bool(lit),..}),
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

    pub(crate) fn impl_attrs(&self) -> Result<Option<String>> {
        self.inner()._optional_str("impl")
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

pub(crate) struct ExceptionItemMeta(ClassItemMeta);

impl ItemMeta for ExceptionItemMeta {
    const ALLOWED_NAMES: &'static [&'static str] = &[
        "module",
        "name",
        "base",
        "unhashable",
        "ctx",
        "impl",
        "traverse",
        "payload",
    ];

    fn from_inner(inner: ItemMetaInner) -> Self {
        Self(ClassItemMeta(inner))
    }

    fn inner(&self) -> &ItemMetaInner {
        &self.0.0
    }
}

impl ExceptionItemMeta {
    pub(crate) fn class_name(&self) -> Result<String> {
        const KEY: &str = "name";
        let inner = self.inner();
        if let Some((_, meta)) = inner.meta_map.get(KEY) {
            match meta {
                Meta::NameValue(syn::MetaNameValue {
                    value:
                        syn::Expr::Lit(syn::ExprLit {
                            lit: syn::Lit::Str(lit),
                            ..
                        }),
                    ..
                }) => return Ok(lit.value()),
                Meta::Path(_) => {
                    return Ok({
                        let type_name = inner.item_name();
                        let Some(py_name) = type_name.as_str().strip_prefix("Py") else {
                            bail_span!(
                                inner.item_ident,
                                "#[pyexception] expects its underlying type to be named `Py` prefixed"
                            )
                        };
                        py_name.to_string()
                    });
                }
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

    pub(crate) fn has_impl(&self) -> Result<bool> {
        self.inner()._bool("impl")
    }
}

impl core::ops::Deref for ExceptionItemMeta {
    type Target = ClassItemMeta;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub(crate) trait AttributeExt: SynAttributeExt {
    fn promoted_nested(&self) -> Result<PunctuatedNestedMeta>;

    fn ident_and_promoted_nested(&self) -> Result<(&Ident, PunctuatedNestedMeta)>;

    fn try_remove_name(&mut self, name: &str) -> Result<Option<NestedMeta>>;

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
                r##"#[{name} = "..."] cannot be a name/value, you probably meant \
                 #[{name}(name = "...")]"##,
            ));
            e
        })?;
        Ok(list.nested)
    }

    fn ident_and_promoted_nested(&self) -> Result<(&Ident, PunctuatedNestedMeta)> {
        Ok((self.get_ident().unwrap(), self.promoted_nested()?))
    }

    fn try_remove_name(&mut self, item_name: &str) -> Result<Option<NestedMeta>> {
        self.try_meta_mut(|meta| {
            let nested = match meta {
                Meta::List(MetaList { nested, .. }) => Ok(nested),
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
                if item.get_ident().is_none_or(|ident| ident != item_name) {
                    continue;
                }

                if found.is_some() {
                    return Err(syn::Error::new(
                        item.span(),
                        format!("#[py..({item_name}...)] must be unique but found multiple times"),
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
                .any(|nested_meta| nested_meta.get_path().is_some_and(|p| p.is_ident(name)));
            if !has_name {
                list.nested.push(new_item())
            }
            Ok(())
        })
    }
}

pub(crate) fn pyclass_ident_and_attrs(item: &syn::Item) -> Result<(&Ident, &[Attribute])> {
    Ok(match item {
        syn::Item::Struct(syn::ItemStruct { ident, attrs, .. }) => (ident, attrs),
        syn::Item::Enum(syn::ItemEnum { ident, attrs, .. }) => (ident, attrs),
        syn::Item::Use(item_use) => (
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

pub(crate) fn pyexception_ident_and_attrs(item: &syn::Item) -> Result<(&Ident, &[Attribute])> {
    Ok(match item {
        syn::Item::Struct(syn::ItemStruct { ident, attrs, .. }) => (ident, attrs),
        syn::Item::Enum(syn::ItemEnum { ident, attrs, .. }) => (ident, attrs),
        other => {
            bail_span!(other, "#[pyexception] can only be on a struct or enum",)
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

enum SigPiece {
    Marker(String),
    Arg {
        name: String,
        ty: Type,
    },
    /// Parameters of the argument a method binds as its receiver.
    Implicit(Type),
}

fn is_vm_or_callee(ty: &Type) -> bool {
    let ty = quote!(#ty).to_string().replace(' ', "");
    (ty.starts_with('&') && ty.ends_with("VirtualMachine")) || ty.ends_with("Callee")
}

fn arg_name(pat: &syn::Pat) -> String {
    match pat {
        syn::Pat::Ident(pat) => {
            let ident = pat.ident.unraw().to_string();
            ident.strip_prefix('_').unwrap_or(&ident).to_owned()
        }
        // `Fildes(fd): Fildes` contributes `fd`. One binding only: a wider
        // pattern has no single parameter name. The name is unused when the
        // type supplies parameters.
        syn::Pat::TupleStruct(pat) if pat.elems.len() == 1 => arg_name(&pat.elems[0]),
        syn::Pat::Reference(pat) => arg_name(&pat.pat),
        syn::Pat::Paren(pat) => arg_name(&pat.pat),
        _ => String::new(),
    }
}

fn mentions_self(ty: &Type) -> bool {
    struct Finder(bool);
    impl Visit<'_> for Finder {
        fn visit_ident(&mut self, ident: &Ident) {
            if ident == "Self" {
                self.0 = true;
            }
        }
    }
    let mut finder = Finder(false);
    finder.visit_type(ty);
    finder.0
}

fn subst_self(ty: &Type, self_ty: Option<&Type>) -> Type {
    struct Subst<'a>(&'a Type);
    impl VisitMut for Subst<'_> {
        fn visit_type_mut(&mut self, ty: &mut Type) {
            if let Type::Path(path) = ty
                && path.qself.is_none()
                && path.path.is_ident("Self")
            {
                *ty = self.0.clone();
                return;
            }
            syn::visit_mut::visit_type_mut(self, ty);
        }
    }
    let Some(self_ty) = self_ty else {
        return ty.clone();
    };
    let mut ty = ty.clone();
    Subst(self_ty).visit_type_mut(&mut ty);
    ty
}

fn sig_pieces(
    sig: &Signature,
    implicit_self: Option<&str>,
    self_ty: Option<&Type>,
    leading_marker: Option<&str>,
) -> Vec<SigPiece> {
    let mut pieces = Vec::new();
    if let Some(marker) = leading_marker {
        pieces.push(SigPiece::Marker(marker.to_owned()));
    }
    let mut implicit_self = implicit_self.map(str::to_owned);
    for arg in &sig.inputs {
        match arg {
            FnArg::Receiver(_) => pieces.push(SigPiece::Marker("$self".to_owned())),
            FnArg::Typed(typed) => {
                if is_vm_or_callee(&typed.ty) {
                    continue;
                }
                let name = arg_name(&typed.pat);
                let ty = subst_self(&typed.ty, self_ty);
                // References carry a lifetime, so `FromArgs` cannot be named
                // from a const item. They are leaf positional parameters.
                let leaf = mentions_self(&ty) || matches!(ty, Type::Reference(_));
                if let Some(marker) = implicit_self.take() {
                    // A nested const cannot name `Self`. The receiver is the
                    // marker; only a type that supplies parameters is kept.
                    pieces.push(SigPiece::Marker(marker));
                    if !leaf {
                        pieces.push(SigPiece::Implicit(ty));
                    }
                } else if leaf {
                    pieces.push(SigPiece::Marker(name));
                } else {
                    pieces.push(SigPiece::Arg { name, ty });
                }
            }
        }
    }
    pieces
}

fn sig_arg_tokens(piece: &SigPiece) -> TokenStream {
    match piece {
        SigPiece::Marker(name) => quote!(::rustpython_vm::function::SigArg::marker(#name)),
        SigPiece::Arg { name, ty } => {
            quote!(::rustpython_vm::function::SigArg::from_arg::<#ty>(#name))
        }
        SigPiece::Implicit(ty) => quote!(::rustpython_vm::function::SigArg::implicit::<#ty>()),
    }
}

fn args_const(pieces: &[SigPiece]) -> TokenStream {
    let args = pieces.iter().map(sig_arg_tokens);
    quote! {
        const ARGS: &[::rustpython_vm::function::SigArg] = &[#(#args),*];
    }
}

/// Expression of type `Option<&'static str>`: the internal doc, or the plain
/// doc when the arguments cannot form a signature.
pub(crate) fn internal_doc_tokens(
    sig: &Signature,
    py_name: &str,
    implicit_self: Option<&str>,
    doc: Option<String>,
    self_ty: Option<&Type>,
    leading_marker: Option<&str>,
) -> TokenStream {
    let args_const = args_const(&sig_pieces(sig, implicit_self, self_ty, leading_marker));
    let plain = match &doc {
        Some(doc) => quote!(Some(#doc)),
        None => quote!(None),
    };
    let doc_text = doc.unwrap_or_default();
    quote! {
        {
            #args_const
            if !::rustpython_vm::function::has_signature(ARGS) {
                #plain
            } else {
                const DOC: &str = #doc_text;
                const N: usize = ::rustpython_vm::function::internal_doc_len(#py_name, ARGS, DOC);
                const B: [u8; N] =
                    ::rustpython_vm::function::internal_doc_bytes::<N>(#py_name, ARGS, DOC);
                const S: &str = match ::core::str::from_utf8(&B) {
                    Ok(s) => s,
                    Err(_) => panic!(),
                };
                Some(S)
            }
        }
    }
}

pub(crate) fn infer_native_call_flags(sig: &Signature, drop_first_typed: usize) -> TokenStream {
    // Best-effort mapping of Rust function signatures to CPython-style
    // METH_* calling convention flags used by CALL specialization.
    let mut typed_args = Vec::new();
    for arg in &sig.inputs {
        let FnArg::Typed(typed) = arg else {
            continue;
        };
        let ty_tokens = &typed.ty;
        let ty = quote!(#ty_tokens).to_string().replace(' ', "");
        // The interpreter supplies `vm` and `callee`; a Python call never
        // passes them.
        if (ty.starts_with('&') && ty.ends_with("VirtualMachine")) || ty.ends_with("Callee") {
            continue;
        }
        typed_args.push(ty);
    }

    let mut user_args = typed_args.into_iter();
    for _ in 0..drop_first_typed {
        if user_args.next().is_none() {
            break;
        }
    }

    let mut has_keywords = false;
    let mut variable_arity = false;
    let mut fixed_positional = 0usize;

    for ty in user_args {
        let is_named = |name: &str| {
            ty == name
                || ty.starts_with(&format!("{name}<"))
                || ty.contains(&format!("::{name}<"))
                || ty.ends_with(&format!("::{name}"))
        };

        if is_named("FuncArgs") {
            has_keywords = true;
            variable_arity = true;
            continue;
        }
        if is_named("KwArgs") {
            has_keywords = true;
            variable_arity = true;
            continue;
        }
        if is_named("PosArgs") || is_named("OptionalArg") || is_named("OptionalOption") {
            variable_arity = true;
            continue;
        }
        fixed_positional += 1;
    }

    if has_keywords {
        quote! {
            rustpython_vm::function::PyMethodFlags::from_bits_retain(
                rustpython_vm::function::PyMethodFlags::FASTCALL.bits()
                    | rustpython_vm::function::PyMethodFlags::KEYWORDS.bits()
            )
        }
    } else if variable_arity {
        quote! { rustpython_vm::function::PyMethodFlags::FASTCALL }
    } else {
        match fixed_positional {
            0 => quote! { rustpython_vm::function::PyMethodFlags::NOARGS },
            1 => quote! { rustpython_vm::function::PyMethodFlags::O },
            _ => quote! { rustpython_vm::function::PyMethodFlags::FASTCALL },
        }
    }
}
