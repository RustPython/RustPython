#![recursion_limit = "128"]
#![doc(html_logo_url = "https://raw.githubusercontent.com/RustPython/RustPython/main/logo.png")]
#![doc(html_root_url = "https://docs.rs/rustpython-derive/")]

use proc_macro::TokenStream;
use rustpython_derive_impl as derive_impl;
use syn::parse_macro_input;

#[proc_macro_derive(FromArgs, attributes(pyarg))]
pub fn derive_from_args(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input);
    derive_impl::derive_from_args(input).into()
}

#[proc_macro_attribute]
pub fn pyclass(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr = parse_macro_input!(attr);
    let item = parse_macro_input!(item);
    derive_impl::pyclass(attr, item).into()
}

/// Helper macro to define `Exception` types.
/// More-or-less is an alias to `pyclass` macro.
///
/// This macro serves a goal of generating multiple
/// `BaseException` / `Exception`
/// subtypes in a uniform and convenient manner.
/// It looks like `SimpleExtendsException` in `CPython`.
/// <https://github.com/python/cpython/blob/main/Objects/exceptions.c>
#[proc_macro_attribute]
pub fn pyexception(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr = parse_macro_input!(attr);
    let item = parse_macro_input!(item);
    derive_impl::pyexception(attr, item).into()
}

#[proc_macro_attribute]
pub fn pymodule(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr = parse_macro_input!(attr);
    let item = parse_macro_input!(item);
    derive_impl::pymodule(attr, item).into()
}

#[proc_macro_derive(PyStructSequence)]
pub fn pystruct_sequence(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input);
    derive_impl::pystruct_sequence(input).into()
}

#[proc_macro_derive(TryIntoPyStructSequence)]
pub fn pystruct_sequence_try_from_object(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input);
    derive_impl::pystruct_sequence_try_from_object(input).into()
}

struct Compiler;
impl derive_impl::Compiler for Compiler {
    fn compile(
        &self,
        source: &str,
        mode: rustpython_compiler::Mode,
        module_name: String,
    ) -> Result<rustpython_compiler::CodeObject, Box<dyn std::error::Error>> {
        use rustpython_compiler::{compile, CompileOpts};
        Ok(compile(source, mode, module_name, CompileOpts::default())?)
    }
}

#[proc_macro]
pub fn py_compile(input: TokenStream) -> TokenStream {
    derive_impl::py_compile(input.into(), &Compiler).into()
}

#[proc_macro]
pub fn py_freeze(input: TokenStream) -> TokenStream {
    derive_impl::py_freeze(input.into(), &Compiler).into()
}

#[proc_macro_derive(PyPayload)]
pub fn pypayload(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input);
    derive_impl::pypayload(input).into()
}

/// use on struct with named fields like `struct A{x:PyRef<B>, y:PyRef<C>}` to impl `Traverse` for datatype.
///
/// use `#[pytraverse(skip)]` on fields you wish not to trace
///
/// add `trace` attr to `#[pyclass]` to make it impl `MaybeTraverse` that will call `Traverse`'s `traverse` method so make it
/// traceable(Even from type-erased PyObject)(i.e. write `#[pyclass(trace)]`).
/// # Example
/// ```rust, ignore
/// #[pyclass(module = false, traverse)]
/// #[derive(Default, Traverse)]
/// pub struct PyList {
///     elements: PyRwLock<Vec<PyObjectRef>>,
///     #[pytraverse(skip)]
///     len: AtomicCell<usize>,
/// }
/// ```
/// This create both `MaybeTraverse` that call `Traverse`'s `traverse` method and `Traverse` that impl `Traverse`
/// for `PyList` which call elements' `traverse` method and ignore `len` field.
#[proc_macro_derive(Traverse, attributes(pytraverse))]
pub fn pytraverse(item: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let item = parse_macro_input!(item);
    derive_impl::pytraverse(item).into()
}
