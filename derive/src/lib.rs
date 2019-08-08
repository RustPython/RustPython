#![recursion_limit = "128"]
#![doc(html_logo_url = "https://raw.githubusercontent.com/RustPython/RustPython/master/logo.png")]
#![doc(html_root_url = "https://docs.rs/rustpython-derive/")]

extern crate proc_macro;

#[macro_use]
extern crate maplit;

#[macro_use]
mod error;
mod compile_bytecode;
mod from_args;
mod pyclass;

use error::{extract_spans, Diagnostic};
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
    let input = parse_macro_input!(input as DeriveInput);
    result_to_tokens(from_args::impl_from_args(input))
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

#[proc_macro_attribute]
pub fn pystruct_sequence(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr = parse_macro_input!(attr as AttributeArgs);
    let item = parse_macro_input!(item as Item);
    result_to_tokens(pyclass::impl_pystruct_sequence(attr, item))
}

#[cfg_attr(feature = "proc-macro-hack", proc_macro_hack::proc_macro_hack)]
#[cfg_attr(not(feature = "proc-macro-hack"), proc_macro)]
pub fn py_compile_bytecode(input: TokenStream) -> TokenStream {
    result_to_tokens(compile_bytecode::impl_py_compile_bytecode(input.into()))
}
