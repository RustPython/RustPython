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
use syn::{parse_macro_input, AttributeArgs, DeriveInput, Item};

fn result_to_tokens(result: Result<TokenStream, impl Into<Diagnostic>>) -> proc_macro::TokenStream {
    result
        .map_err(|e| e.into())
        .unwrap_or_else(ToTokens::into_token_stream)
        .into()
}

#[proc_macro_derive(FromArgs, attributes(pyarg))]
pub fn derive_from_args(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    result_to_tokens(from_args::impl_from_args(input))
}

#[proc_macro_attribute]
pub fn pyclass(
    attr: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let attr = parse_macro_input!(attr as AttributeArgs);
    let item = parse_macro_input!(item as Item);
    if matches!(item, syn::Item::Impl(_) | syn::Item::Trait(_)) {
        result_to_tokens(pyclass::impl_pyimpl(attr, item))
    } else {
        result_to_tokens(pyclass::impl_pyclass(attr, item))
    }
}

/// This macro serves a goal of generating multiple
/// `BaseException` / `Exception`
/// subtypes in a uniform and convenient manner.
/// It looks like `SimpleExtendsException` in `CPython`.
/// <https://github.com/python/cpython/blob/main/Objects/exceptions.c>
///
/// We need `ctx` to be ready to add
/// `properties` / `custom` constructors / slots / methods etc.
/// So, we use `extend_class!` macro as the second
/// step in exception type definition.
#[proc_macro]
pub fn define_exception(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let exc_def = parse_macro_input!(input as pyclass::PyExceptionDef);
    result_to_tokens(pyclass::impl_define_exception(exc_def))
}

/// Helper macro to define `Exception` types.
/// More-or-less is an alias to `pyclass` macro.
#[proc_macro_attribute]
pub fn pyexception(
    attr: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let attr = parse_macro_input!(attr as AttributeArgs);
    let item = parse_macro_input!(item as Item);
    result_to_tokens(pyclass::impl_pyexception(attr, item))
}

#[proc_macro_attribute]
pub fn pymodule(
    attr: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let attr = parse_macro_input!(attr as AttributeArgs);
    let item = parse_macro_input!(item as Item);
    result_to_tokens(pymodule::impl_pymodule(attr, item))
}

#[proc_macro_derive(PyStructSequence)]
pub fn pystruct_sequence(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    result_to_tokens(pystructseq::impl_pystruct_sequence(input))
}

#[proc_macro_derive(TryIntoPyStructSequence)]
pub fn pystruct_sequence_try_from_object(
    input: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    result_to_tokens(pystructseq::impl_pystruct_sequence_try_from_object(input))
}

// would be cool to move all the macro implementation to a separate rustpython-derive-shared
// that just depends on rustpython-compiler-core, and then rustpython-derive would hook -compiler
// up to it; so that (the bulk of) rustpython-derive and rustpython-codegen could build in parallel
struct Compiler;
impl compile_bytecode::Compiler for Compiler {
    fn compile(
        &self,
        source: &str,
        mode: rustpython_compiler_core::Mode,
        module_name: String,
    ) -> Result<rustpython_compiler_core::CodeObject, Box<dyn std::error::Error>> {
        use rustpython_compiler::{compile, CompileOpts};
        Ok(compile(source, mode, module_name, CompileOpts::default())?)
    }
}

#[proc_macro]
pub fn py_compile(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    result_to_tokens(compile_bytecode::impl_py_compile(input.into(), &Compiler))
}

#[proc_macro]
pub fn py_freeze(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    result_to_tokens(compile_bytecode::impl_py_freeze(input.into(), &Compiler))
}

#[proc_macro_derive(PyPayload)]
pub fn pypayload(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    result_to_tokens(pypayload::impl_pypayload(input))
}
