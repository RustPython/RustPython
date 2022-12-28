#![recursion_limit = "128"]
#![doc(html_logo_url = "https://raw.githubusercontent.com/RustPython/RustPython/main/logo.png")]
#![doc(html_root_url = "https://docs.rs/rustpython-derive/")]

extern crate proc_macro;

#[macro_use]
extern crate maplit;

#[macro_use]
mod error;
#[macro_use]
mod util;

mod compile_bytecode;
mod from_args;
mod pyclass;
mod pymodule;
mod pypayload;
mod pystructseq;

use error::{extract_spans, Diagnostic};
use proc_macro2::TokenStream;
use quote::ToTokens;
use rustpython_doc as doc;
use syn::{AttributeArgs, DeriveInput, Item};

pub use compile_bytecode::Compiler;

fn result_to_tokens(result: Result<TokenStream, impl Into<Diagnostic>>) -> TokenStream {
    result
        .map_err(|e| e.into())
        .unwrap_or_else(ToTokens::into_token_stream)
}

pub fn derive_from_args(input: DeriveInput) -> TokenStream {
    result_to_tokens(from_args::impl_from_args(input))
}

pub fn pyclass(attr: AttributeArgs, item: Item) -> TokenStream {
    if matches!(item, syn::Item::Impl(_) | syn::Item::Trait(_)) {
        result_to_tokens(pyclass::impl_pyimpl(attr, item))
    } else {
        result_to_tokens(pyclass::impl_pyclass(attr, item))
    }
}

pub use pyclass::PyExceptionDef;
pub fn define_exception(exc_def: PyExceptionDef) -> TokenStream {
    result_to_tokens(pyclass::impl_define_exception(exc_def))
}

pub fn pyexception(attr: AttributeArgs, item: Item) -> TokenStream {
    result_to_tokens(pyclass::impl_pyexception(attr, item))
}

pub fn pymodule(attr: AttributeArgs, item: Item) -> TokenStream {
    result_to_tokens(pymodule::impl_pymodule(attr, item))
}

pub fn pystruct_sequence(input: DeriveInput) -> TokenStream {
    result_to_tokens(pystructseq::impl_pystruct_sequence(input))
}

pub fn pystruct_sequence_try_from_object(input: DeriveInput) -> TokenStream {
    result_to_tokens(pystructseq::impl_pystruct_sequence_try_from_object(input))
}

pub fn py_compile(input: TokenStream, compiler: &dyn Compiler) -> TokenStream {
    result_to_tokens(compile_bytecode::impl_py_compile(input, compiler))
}

pub fn py_freeze(input: TokenStream, compiler: &dyn Compiler) -> TokenStream {
    result_to_tokens(compile_bytecode::impl_py_freeze(input, compiler))
}

pub fn pypayload(input: DeriveInput) -> TokenStream {
    result_to_tokens(pypayload::impl_pypayload(input))
}
