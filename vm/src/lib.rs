//! This crate contains most python logic.
//!
//! - Compilation
//! - Bytecode
//! - Import mechanics
//! - Base objects

// for methods like vm.to_str(), not the typical use of 'to' as a method prefix
#![allow(clippy::wrong_self_convention, clippy::implicit_hasher)]
#![doc(html_logo_url = "https://raw.githubusercontent.com/RustPython/RustPython/master/logo.png")]
#![doc(html_root_url = "https://docs.rs/rustpython-vm/")]
#![cfg_attr(
    target_os = "redox",
    feature(matches_macro, proc_macro_hygiene, result_map_or)
)]

#[cfg(feature = "flame-it")]
#[macro_use]
extern crate flamer;

#[macro_use]
extern crate bitflags;
#[macro_use]
extern crate log;
#[macro_use]
extern crate maplit;
// extern crate env_logger;

#[macro_use]
extern crate rustpython_derive;

extern crate self as rustpython_vm;

pub use rustpython_derive::*;

//extern crate eval; use eval::eval::*;
// use py_code_object::{Function, NativeType, PyCodeObject};

// This is above everything else so that the defined macros are available everywhere
#[macro_use]
pub mod macros;

mod builtins;
mod bytesinner;
pub mod byteslike;
pub mod cformat;
mod dictdatatype;
#[cfg(feature = "rustpython-compiler")]
pub mod eval;
pub mod exceptions;
pub mod format;
pub mod frame;
mod frozen;
pub mod function;
pub mod import;
pub mod obj;
mod py_io;
pub mod py_serde;
pub mod pyobject;
mod pystr;
pub mod readline;
pub mod scope;
mod sequence;
pub mod slots;
pub mod stdlib;
mod sysmodule;
pub mod types;
pub mod util;
mod version;
mod vm;

// pub use self::pyobject::Executor;
pub use self::vm::{InitParameter, PySettings, VirtualMachine};
pub use rustpython_bytecode::*;
pub use rustpython_common as common;

#[doc(hidden)]
pub mod __exports {
    pub use smallbox::smallbox;
}
