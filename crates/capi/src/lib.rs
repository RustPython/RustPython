#![allow(clippy::missing_safety_doc)]
pub use rustpython_vm::PyObject;

extern crate alloc;

pub mod pylifecycle;
pub mod pystate;
pub mod refcount;
