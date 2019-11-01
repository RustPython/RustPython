//! This crate contains most python logic.
//!
//! - Compilation
//! - Bytecode
//! - Import mechanics
//! - Base objects

// for methods like vm.to_str(), not the typical use of 'to' as a method prefix
#![allow(
    clippy::wrong_self_convention,
    clippy::let_and_return,
    clippy::implicit_hasher
)]
#![doc(html_logo_url = "https://raw.githubusercontent.com/RustPython/RustPython/master/logo.png")]
#![doc(html_root_url = "https://docs.rs/rustpython-vm/")]
#![cfg_attr(not(feature = "use-proc-macro-hack"), feature(proc_macro_hygiene))]

#[cfg(feature = "flame-it")]
#[macro_use]
extern crate flamer;

#[macro_use]
extern crate bitflags;
#[macro_use]
extern crate lazy_static;
extern crate lexical;
#[macro_use]
extern crate log;
#[macro_use]
extern crate maplit;
// extern crate env_logger;

#[macro_use]
extern crate rustpython_derive;

extern crate self as rustpython_vm;

pub use rustpython_derive::*;

#[cfg(feature = "use-proc-macro-hack")]
#[proc_macro_hack::proc_macro_hack]
pub use rustpython_derive::py_compile_bytecode;

//extern crate eval; use eval::eval::*;
// use py_code_object::{Function, NativeType, PyCodeObject};

// This is above everything else so that the defined macros are available everywhere
#[macro_use]
pub mod macros;

mod builtins;
pub mod cformat;
mod dictdatatype;
#[cfg(feature = "rustpython-compiler")]
pub mod eval;
mod exceptions;
pub mod format;
mod frame;
mod frozen;
pub mod function;
pub mod import;
pub mod obj;
pub mod py_serde;
mod pyhash;
pub mod pyobject;
pub mod scope;
pub mod stdlib;
mod sysmodule;
pub mod types;
pub mod util;
mod version;
mod vm;

// pub use self::pyobject::Executor;
pub use self::exceptions::{print_exception, write_exception};
pub use self::vm::{PySettings, VirtualMachine};
pub use rustpython_bytecode::*;

#[doc(hidden)]
pub mod __exports {
    pub use maplit::hashmap;
}
