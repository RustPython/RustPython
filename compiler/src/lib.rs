//! Compile a Python AST or source code into bytecode consumable by RustPython.
#![doc(html_logo_url = "https://raw.githubusercontent.com/RustPython/RustPython/master/logo.png")]
#![doc(html_root_url = "https://docs.rs/rustpython-compiler/")]

#[macro_use]
extern crate log;

type IndexMap<K, V> = indexmap::IndexMap<K, V, ahash::RandomState>;
type IndexSet<T> = indexmap::IndexSet<T, ahash::RandomState>;

pub mod compile;
pub mod error;
pub mod ir;
pub mod mode;
pub mod symboltable;
