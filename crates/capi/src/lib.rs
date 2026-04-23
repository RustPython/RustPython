#![allow(clippy::missing_safety_doc)]

use crate::pylifecycle::MAIN_INTERP;
use rustpython_vm::Interpreter;
pub use rustpython_vm::PyObject;
use std::sync::MutexGuard;

extern crate alloc;

pub mod pylifecycle;
pub mod pystate;
pub mod refcount;

/// Get main interpreter of this process. Will be None if it has not been initialized yet.
pub fn get_main_interpreter() -> MutexGuard<'static, Option<Interpreter>> {
    MAIN_INTERP
        .lock()
        .expect("Failed to lock interpreter mutex")
}
