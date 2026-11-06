//! The item information shared by module, impl, and trait macro expansion.

use proc_macro2::Span;
use quote::ToTokens;
use syn::{Attribute, Ident, Result, Signature, spanned::Spanned};

pub(crate) struct Function<'a> {
    signature: &'a Signature,
    span: Span,
}

impl<'a> Function<'a> {
    pub(crate) fn sig(&self) -> &'a Signature {
        self.signature
    }

    pub(crate) fn span(&self) -> Span {
        self.span
    }
}

pub(crate) struct Constant<'a> {
    ident: &'a Ident,
    span: Span,
}

impl<'a> Constant<'a> {
    pub(crate) fn ident(&self) -> &'a Ident {
        self.ident
    }

    pub(crate) fn span(&self) -> Span {
        self.span
    }
}

pub(crate) trait ItemLike: ToTokens {
    fn attrs_mut(&mut self) -> Result<&mut Vec<Attribute>>;
    fn function_or_method(&self) -> Result<Function<'_>>;
    fn constant(&self) -> Result<Constant<'_>>;

    // Only functions and constants can supply values to #[pyattr].
    fn get_ident(&self) -> Option<&Ident> {
        if let Ok(function) = self.function_or_method() {
            Some(&function.sig().ident)
        } else {
            self.constant().ok().map(|constant| constant.ident())
        }
    }

    fn try_split_attr_mut<R>(
        &mut self,
        expand: impl FnOnce(&mut Vec<Attribute>, &mut Self) -> Result<R>,
    ) -> Result<R>
    where
        Self: Sized,
    {
        let mut attrs = core::mem::take(self.attrs_mut()?);
        let result = expand(&mut attrs, self);
        // Restore attributes even when expansion reports a diagnostic.
        let item_attrs = self.attrs_mut()?;
        assert!(
            item_attrs.is_empty(),
            "attrs changed during replacement. this behavior must be a bug."
        );
        *item_attrs = attrs;
        result
    }
}

macro_rules! impl_item {
    ($item:ident, $attrs_error:literal, $($variant:ident),+ $(,)?) => {
        impl ItemLike for syn::$item {
            fn attrs_mut(&mut self) -> Result<&mut Vec<Attribute>> {
                match self {
                    $(Self::$variant(item) => Ok(&mut item.attrs),)+
                    other => Err(syn::Error::new_spanned(other, $attrs_error)),
                }
            }

            fn function_or_method(&self) -> Result<Function<'_>> {
                match self {
                    Self::Fn(item) => Ok(Function {
                        signature: &item.sig,
                        span: item.span(),
                    }),
                    other => Err(syn::Error::new_spanned(
                        other,
                        "this item is not a function or method",
                    )),
                }
            }

            fn constant(&self) -> Result<Constant<'_>> {
                match self {
                    Self::Const(item) => Ok(Constant {
                        ident: &item.ident,
                        span: item.span(),
                    }),
                    other => Err(syn::Error::new_spanned(other, "this item is not a const")),
                }
            }
        }
    };
}

impl_item!(
    Item,
    "this kind of item doesn't have attrs",
    Const,
    Enum,
    ExternCrate,
    Fn,
    ForeignMod,
    Impl,
    Macro,
    Mod,
    Static,
    Struct,
    Trait,
    TraitAlias,
    Type,
    Union,
    Use,
);
impl_item!(
    ImplItem,
    "this kind of item doesn't have attrs",
    Const,
    Fn,
    Type,
    Macro,
);
impl_item!(
    TraitItem,
    "this kind of impl item doesn't have attrs",
    Const,
    Fn,
    Type,
    Macro,
);
