//! Compile a Python AST or source code into bytecode consumable by RustPython or
//! (eventually) CPython.

#[macro_use]
extern crate log;

pub mod compile;
pub mod error;
mod symboltable;
