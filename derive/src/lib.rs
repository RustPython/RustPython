extern crate proc_macro;

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Data, DeriveInput, Fields};

#[proc_macro_derive(FromArgs)]
pub fn derive_from_args(input: TokenStream) -> TokenStream {
    let ast: DeriveInput = syn::parse(input).unwrap();

    let gen = impl_from_args(&ast);
    gen.to_string().parse().unwrap()
}

fn impl_from_args(input: &DeriveInput) -> TokenStream2 {
    // FIXME: This references types using `crate` instead of `rustpython_vm`
    //        so that it can be used in the latter. How can we support both?
    let fields = match input.data {
        Data::Struct(ref data) => {
            match data.fields {
                Fields::Named(ref fields) => fields.named.iter().map(|field| {
                    let name = &field.ident;
                    quote! {
                        #name: crate::pyobject::TryFromObject::try_from_object(
                            vm,
                            args.take_keyword(stringify!(#name)).unwrap_or_else(|| vm.ctx.none())
                        )?,
                    }
                }),
                Fields::Unnamed(_) | Fields::Unit => unimplemented!(), // TODO: better error message
            }
        }
        Data::Enum(_) | Data::Union(_) => unimplemented!(), // TODO: better error message
    };

    let name = &input.ident;
    quote! {
        impl crate::function::FromArgs for #name {
            fn from_args(
                vm: &crate::vm::VirtualMachine,
                args: &mut crate::function::PyFuncArgs
            ) -> Result<Self, crate::function::ArgumentError> {
                Ok(#name { #(#fields)* })
            }
        }
    }
}
