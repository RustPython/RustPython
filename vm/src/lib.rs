#[macro_use]
extern crate log;
// extern crate env_logger;

//extern crate eval; use eval::eval::*;
// use py_code_object::{Function, NativeType, PyCodeObject};

mod builtins;
pub mod bytecode;
pub mod compile;
pub mod eval;
mod frame;
mod import;
mod objbool;
mod objdict;
mod objint;
mod objlist;
mod objsequence;
mod objstr;
mod objtype;
pub mod pyobject;
mod sysmodule;
mod vm;

// pub use self::pyobject::Executor;
pub use self::vm::VirtualMachine;
