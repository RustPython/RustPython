use quote::quote;
use syn::{Error, Ident, ItemEnum, ItemStruct, spanned::Spanned};

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
    let output = quote! {
        #(#attrs)*
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
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

struct VariantInfo {
    ident: Ident,
    discriminant: Option<syn::Expr>,
    display: Option<String>,
    catch_all: bool,
}

impl TryFrom<syn::Variant> for VariantInfo {
    type Error = syn::Error;

    fn try_from(variant: syn::Variant) -> Result<Self, Self::Error> {
        let mut display = None;
        let mut catch_all = false;
        for attr in &variant.attrs {
            if !attr.path().is_ident("oparg") {
                continue;
            }

            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("display") {
                    let value = meta.value()?.parse::<syn::LitStr>()?;
                    display = Some(value.value());
                    Ok(())
                } else if meta.path.is_ident("catch_all") {
                    catch_all = true;
                    Ok(())
                } else {
                    Err(meta.error("unknown oparg attribute"))
                }
            })?
        }

        let ident = variant.ident.clone();
        let discriminant = variant.discriminant.as_ref().map(|(_, expr)| expr.clone());

        if catch_all && display.is_some() {
            return Err(Error::new(
                ident.span(),
                r#"Cannot define both `#[oparg(catch_all)`] and `#[oparg(display = "...")]` on the same variant"#,
            ));
        }

        if discriminant.is_none() && !catch_all {
            return Err(Error::new(
                ident.span(),
                "Is a variant without an assigned value",
            ));
        }

        Ok(Self {
            ident,
            discriminant,
            display,
            catch_all,
        })
    }
}

pub(super) fn handle_enum(item: ItemEnum) -> syn::Result<proc_macro2::TokenStream> {
    if !item.generics.params.is_empty() {
        return Err(Error::new(
            item.span(),
            "A new type oparg cannot be generic.",
        ));
    }

    let ItemEnum {
        attrs,
        vis,
        enum_token,
        ident,
        generics: _,
        brace_token: _,
        variants,
    } = item.clone();

    let mut variants_info = variants
        .iter()
        .cloned()
        .map(VariantInfo::try_from)
        .collect::<syn::Result<Vec<_>>>()?;

    let catch_all = variants_info.pop_if(|info| info.catch_all);

    // Ensure a no multiple `#[oparg(catch_all)]`
    if catch_all.is_some() && variants_info.iter().any(|vinfo| vinfo.catch_all) {
        return Err(Error::new(
            item.span(),
            "Cannot define more than one `#[oparg(catch_all)]`",
        ));
    };

    let variants_def = variants.iter().cloned().map(|mut variant| {
        // Don't assign value. Enables more optimizations by the compiler.
        variant.discriminant = None;

        // Remove `#[oparg(...)`.
        variant.attrs.retain(|attr| !attr.path().is_ident("oparg"));

        variant
    });

    let from_u32_arms = variants_info.iter().map(|vinfo| {
        let ident = &vinfo.ident;
        let discriminant = &vinfo.discriminant;

        quote! {
            #discriminant => Self::#ident,
        }
    });

    // If we have a `catch_all` we can implement `From<u32>`. Otherwise impl `TryFrom<u32>`
    let impl_from_u32 = match catch_all {
        Some(ref vinfo) => {
            let vinfo_ident = &vinfo.ident;
            quote! {
                impl From<u32> for #ident {
                    fn from(value: u32) -> Self {
                        match value {
                            #(#from_u32_arms)*
                            _ => Self::#vinfo_ident(value)
                        }
                    }
                }
            }
        }
        None => quote! {
            impl TryFrom<u32> for #ident {
                type Error = rustpython_compiler_core::marshal::MarshalError;

                fn try_from(value: u32) -> Result<Self, Self::Error> {
                    Ok(
                        match value {
                            #(#from_u32_arms)*
                            _ => return Err(Self::Error::InvalidBytecode),
                        }
                    )
                }
            }
        },
    };

    let mut into_u32_arms = vec![];
    let mut display_arms = vec![];

    for vinfo in &variants_info {
        let VariantInfo {
            ident: vinfo_ident,
            discriminant,
            display,
            ..
        } = &vinfo;

        into_u32_arms.push(quote! {
            #ident::#vinfo_ident => #discriminant,
        });

        let display_arm = match display {
            Some(v) => quote! {
                Self::#vinfo_ident => write!(f, "{}", #v),
            },
            None => quote! {
                Self::#vinfo_ident => write!(f, "{}", #discriminant),
            },
        };

        display_arms.push(display_arm);
    }

    if let Some(ref vinfo) = catch_all {
        let vinfo_ident = &vinfo.ident;

        into_u32_arms.push(quote! {
            #ident::#vinfo_ident(v) => v,
        });

        display_arms.push(quote! {
            Self::#vinfo_ident(v) => write!(f, "{}", v),
        });
    }

    let output = quote! {
        #(#attrs)*
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        #vis #enum_token #ident {
            #(#variants_def),*
        }

        #impl_from_u32

        impl From<#ident> for u32 {
            fn from(value: #ident) -> Self {
                match value {
                    #(#into_u32_arms)*
                }
            }
        }

        impl ::core::fmt::Display for #ident {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                match self {
                    #(#display_arms)*
                }
            }
        }

        impl rustpython_compiler_core::bytecode::OpArgType for #ident {}
    };

    Ok(output)
}
