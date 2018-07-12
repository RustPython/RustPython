#[macro_use]
extern crate log;
extern crate env_logger;

//extern crate eval; use eval::eval::*;
// use py_code_object::{Function, NativeType, PyCodeObject};

pub mod pyobject;
pub mod bytecode;
mod builtins;
mod vm;
mod objint;
mod objtype;
pub mod compile;

pub use self::vm::VirtualMachine;
