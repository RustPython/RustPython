//! Compile a Python AST or source code into bytecode consumable by RustPython or
//! (eventually) CPython.
#![doc(html_logo_url = "https://raw.githubusercontent.com/RustPython/RustPython/master/logo.png")]
#![doc(html_root_url = "https://docs.rs/rustpython-compiler/")]

#[macro_use]
extern crate log;

pub mod compile;
pub mod error;
pub mod mode;
pub(crate) mod output_stream;
pub mod peephole;
pub mod symboltable;
