#![allow(clippy::missing_safety_doc)]

use crate::pyerrors::init_exception_statics;
use crate::pylifecycle::MAIN_INTERP;
pub use rustpython_vm::PyObject;
use rustpython_vm::{Context, Interpreter};
use std::sync::MutexGuard;

extern crate alloc;

pub mod abstract_;
pub mod import;
pub mod object;
pub mod pyerrors;
pub mod pylifecycle;
pub mod pystate;
pub mod refcount;
mod util;

/// Get main interpreter of this process. Will be None if it has not been initialized yet.
pub fn get_main_interpreter() -> MutexGuard<'static, Option<Interpreter>> {
    MAIN_INTERP
        .lock()
        .expect("Failed to lock interpreter mutex")
}

/// Set the main interpreter of this process. This method will panic when there is already an
/// interpreter set.
pub fn init_main_interpreter(interpreter: Interpreter) {
    let mut interp = get_main_interpreter();
    assert!(interp.is_none(), "Main interpreter is already set");
    // Safety: Interpreter was not initialized before, so we can safely assume the statics are not used
    unsafe { init_exception_statics(&Context::genesis().exceptions) };
    *interp = Some(interpreter);
}
