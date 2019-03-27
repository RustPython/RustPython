extern crate proc_macro;

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Data, DeriveInput, Field, Fields};

#[proc_macro_derive(FromArgs, attributes(positional, keyword))]
pub fn derive_from_args(input: TokenStream) -> TokenStream {
    let ast: DeriveInput = syn::parse(input).unwrap();

    let gen = impl_from_args(&ast);
    gen.to_string().parse().unwrap()
}

enum ArgType {
    Positional,
    PositionalKeyword,
    Keyword,
}

fn generate_field(field: &Field) -> TokenStream2 {
    let arg_type = if let Some(attr) = field.attrs.first() {
        if attr.path.is_ident("positional") {
            ArgType::Positional
        } else if attr.path.is_ident("keyword") {
            ArgType::Keyword
        } else {
            panic!("Unrecognised attribute")
        }
    } else {
        ArgType::PositionalKeyword
    };

    let name = &field.ident;
    match arg_type {
        ArgType::Positional => {
            quote! {
                #name: args.take_positional(vm)?,
            }
        }
        ArgType::PositionalKeyword => {
            quote! {
                #name: args.take_positional_keyword(vm, stringify!(#name))?,
            }
        }
        ArgType::Keyword => {
            quote! {
                #name: args.take_keyword(vm, stringify!(#name))?,
            }
        }
    }
}

fn impl_from_args(input: &DeriveInput) -> TokenStream2 {
    // FIXME: This references types using `crate` instead of `rustpython_vm`
    //        so that it can be used in the latter. How can we support both?
    //        Can use extern crate self as rustpython_vm; once in stable.
    //        https://github.com/rust-lang/rust/issues/56409
    let fields = match input.data {
        Data::Struct(ref data) => {
            match data.fields {
                Fields::Named(ref fields) => fields.named.iter().map(generate_field),
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
