#[macro_use]
extern crate bitflags;
#[macro_use]
extern crate log;
// extern crate env_logger;
extern crate serde;
extern crate serde_json;

//extern crate eval; use eval::eval::*;
// use py_code_object::{Function, NativeType, PyCodeObject};

// This is above everything else so that the defined macros are available everywhere
#[macro_use]
mod macros;

mod builtins;
pub mod bytecode;
pub mod compile;
pub mod eval;
mod exceptions;
mod frame;
mod import;
mod objbool;
mod objdict;
mod objfloat;
mod objfunction;
mod objint;
mod objlist;
mod objobject;
mod objsequence;
mod objstr;
mod objtype;
pub mod pyobject;
pub mod stdlib;
mod sysmodule;
mod traceback;
mod vm;

// pub use self::pyobject::Executor;
pub use self::vm::VirtualMachine;
pub use self::exceptions::print_exception;
