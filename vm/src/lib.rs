#[macro_use]
extern crate log;
extern crate env_logger;

//extern crate eval; use eval::eval::*;
// use py_code_object::{Function, NativeType, PyCodeObject};

mod pyobject;
pub mod bytecode;
mod builtins;
mod vm;

pub use self::vm::evaluate;
