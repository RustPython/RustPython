//! This crate contains most python logic.
//!
//! - Compilation
//! - Bytecode
//! - Import mechanics
//! - Base objects

// to allow `mod foo {}` in foo.rs; clippy thinks this is a mistake/misunderstanding of
// how `mod` works, but we want this sometimes for pymodule declarations
#![allow(clippy::module_inception)]
// we want to mirror python naming conventions when defining python structs, so that does mean
// uppercase acronyms, e.g. TextIOWrapper instead of TextIoWrapper
#![allow(clippy::upper_case_acronyms)]
#![doc(html_logo_url = "https://raw.githubusercontent.com/RustPython/RustPython/main/logo.png")]
#![doc(html_root_url = "https://docs.rs/rustpython-vm/")]

#[cfg(feature = "flame-it")]
#[macro_use]
extern crate flamer;

#[macro_use]
extern crate bitflags;
#[macro_use]
extern crate log;
// extern crate env_logger;

#[macro_use]
extern crate rustpython_derive;

extern crate self as rustpython_vm;

pub use rustpython_derive::*;

//extern crate eval; use eval::eval::*;
// use py_code_object::{Function, NativeType, PyCodeObject};

// This is above everything else so that the defined macros are available everywhere
#[macro_use]
pub(crate) mod macros;

mod anystr;
pub mod buffer;
pub mod builtins;
pub mod byte;
mod bytesinner;
pub mod cformat;
pub mod class;
mod codecs;
pub mod compiler;
pub mod convert;
mod coroutine;
mod dictdatatype;
#[cfg(feature = "rustpython-compiler")]
pub mod eval;
pub mod exceptions;
pub mod format;
pub mod frame;
mod frozen;
pub mod function;
pub mod import;
mod intern;
pub mod iter;
pub mod object;
pub mod prelude;
pub mod protocol;
pub mod py_io;
pub mod py_serde;
pub mod readline;
pub mod recursion;
pub mod scope;
pub mod sequence;
pub mod signal;
pub mod sliceable;
pub mod stdlib;
pub mod suggestion;
pub mod types;
pub mod utils;
pub mod version;
pub mod vm;
pub mod warn;

pub use self::convert::{TryFromBorrowedObject, TryFromObject};
pub use self::object::{
    AsObject, Py, PyAtomicRef, PyExact, PyObject, PyObjectAtomicRef, PyObjectRef, PyPayload, PyRef,
    PyRefExact, PyResult, PyWeakRef,
};
pub use self::vm::{Context, Interpreter, Settings, VirtualMachine};

pub use rustpython_common as common;
pub use rustpython_compiler_core as bytecode;

#[doc(hidden)]
pub mod __exports {
    pub use paste;
}
