//! This crate contains most python logic.
//!
//! - Compilation
//! - Bytecode
//! - Import mechanics
//! - Base objects

// for methods like vm.to_str(), not the typical use of 'to' as a method prefix
#![allow(clippy::wrong_self_convention)]
// to allow `mod foo {}` in foo.rs; clippy thinks this is a mistake/misunderstanding of
// how `mod` works, but we want this sometimes for pymodule declarations
#![allow(clippy::module_inception)]
// to encourage good API design, see https://github.com/rust-lang/rust-clippy/issues/6726
#![allow(clippy::unnecessary_wraps)]
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
pub mod builtins;
mod bytesinner;
pub mod cformat;
mod codecs;
mod coroutine;
#[cfg(any(unix, windows, target_os = "wasi"))]
mod crt_fd;
mod dictdatatype;
#[cfg(feature = "rustpython-compiler")]
pub mod eval;
pub mod exceptions;
pub mod format;
pub mod frame;
mod frozen;
pub mod function;
pub mod import;
pub mod protocol;
pub mod py_io;
pub mod py_serde;
mod pyobject;
mod pyobjectrc;
pub mod readline;
pub mod scope;
pub mod sequence;
pub mod signal;
pub mod sliceable;
pub mod stdlib;
pub mod suggestion;
pub mod types;
pub mod utils;
pub mod version;
mod vm;

// pub use self::Executor;
pub use self::pyobject::*;
pub use self::vm::{InitParameter, Interpreter, PySettings, VirtualMachine};
pub use rustpython_bytecode as bytecode;
pub use rustpython_common as common;
#[cfg(feature = "rustpython-compiler")]
pub use rustpython_compiler as compile;

#[doc(hidden)]
pub mod __exports {
    pub use paste;
}
