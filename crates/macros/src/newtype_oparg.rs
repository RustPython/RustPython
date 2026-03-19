use syn::{Error, ItemEnum, ItemStruct, spanned::Spanned};

pub(super) fn handle_struct(item: ItemStruct) -> syn::Result<proc_macro2::TokenStream> {
    if !item.fields.is_empty() {
        return Err(Error::new(
            item.span(),
            "A new type oparg cannot have any fields.",
        ));
    }

    if !item.generics.params.is_empty() {
        return Err(Error::new(
            item.span(),
            "A new type oparg cannot be generic.",
        ));
    }

    let ItemStruct {
        attrs,
        vis,
        struct_token,
        ident,
        generics: _,
        fields: _,
        semi_token,
    } = item;

    let semi_token = semi_token.unwrap_or_default();
    let output = quote::quote! {
        #(#attrs)*
        #[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
        #vis #struct_token #ident(u32)#semi_token

        impl #ident {
            #[must_use]
            #vis const fn new(value: u32) -> Self {
                Self::from_u32(value)
            }

            #[must_use]
            #vis const fn from_u32(value: u32) -> Self {
                Self(value)
            }

            /// Returns the oparg as a `u32` value.
            #[must_use]
            #vis const fn as_u32(self) -> u32 {
                self.0
            }

            /// Returns the oparg as a `usize` value.
            #[must_use]
            #vis const fn as_usize(self) -> usize {
                self.0 as usize
            }
        }

        impl From<u32> for #ident {
            fn from(value: u32) -> Self {
                Self::from_u32(value)
            }
        }

        impl From<#ident> for u32 {
            fn from(value: #ident) -> Self {
                value.0
            }
        }

        impl From<#ident> for usize {
            fn from(value: #ident) -> Self {
                value.as_usize()
            }
        }

        impl ::core::fmt::Display for #ident {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                self.0.fmt(f)
            }
        }

        impl rustpython_compiler_core::bytecode::OpArgType for #ident {}
    };

    Ok(output)
}

pub(super) fn handle_enum(item: ItemEnum) -> syn::Result<proc_macro2::TokenStream> {
    let ItemEnum {
        attrs,
        vis,
        enum_token,
        ident,
        generics: _,
        fields: _,
        variants,
    } = item;

    let output = quote::quote! {
        #(#attrs)*
        #[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
        #vis #enum_token #ident {
        }
    };

    Ok(output)
}
