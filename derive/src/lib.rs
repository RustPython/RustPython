extern crate proc_macro;

#[macro_use]
mod error;
mod from_args;
mod pyclass;

use error::Diagnostic;
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::ToTokens;
use syn::{parse_macro_input, AttributeArgs, DeriveInput, Item};

fn result_to_tokens(result: Result<TokenStream2, Diagnostic>) -> TokenStream {
    match result {
        Ok(tokens) => tokens.into(),
        Err(diagnostic) => diagnostic.into_token_stream().into(),
    }
}

#[proc_macro_derive(FromArgs, attributes(pyarg))]
pub fn derive_from_args(input: TokenStream) -> TokenStream {
    let ast: DeriveInput = syn::parse(input).unwrap();

    from_args::impl_from_args(ast).into()
}

#[proc_macro_attribute]
pub fn pyclass(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr = parse_macro_input!(attr as AttributeArgs);
    let item = parse_macro_input!(item as Item);
    result_to_tokens(pyclass::impl_pyclass(attr, item))
}

#[proc_macro_attribute]
pub fn pyimpl(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr = parse_macro_input!(attr as AttributeArgs);
    let item = parse_macro_input!(item as Item);
    result_to_tokens(pyclass::impl_pyimpl(attr, item))
}
