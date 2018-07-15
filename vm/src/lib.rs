#[macro_use]
extern crate log;
extern crate env_logger;

//extern crate eval; use eval::eval::*;
// use py_code_object::{Function, NativeType, PyCodeObject};

mod builtins;
pub mod bytecode;
pub mod compile;
pub mod eval;
mod frame;
mod objint;
mod objlist;
mod objstr;
mod objtuple;
mod objtype;
pub mod pyobject;
mod vm;

pub use self::vm::VirtualMachine;
