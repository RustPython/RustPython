use crate::pystate::attach_vm_to_thread;
use core::ffi::c_int;
use rustpython_vm::Interpreter;
use rustpython_vm::vm::thread::ThreadedVirtualMachine;
use std::sync::{Mutex, OnceLock};

static MAIN_INTERP: OnceLock<Mutex<Interpreter>> = OnceLock::new();

/// Request a thread local vm from the main interpreter
pub(crate) fn request_vm_from_interpreter() -> ThreadedVirtualMachine {
    MAIN_INTERP
        .get()
        .expect("Interpreter is not initialized")
        .lock()
        .expect("Failed to lock interpreter mutex")
        .enter(|vm| vm.new_thread())
}

#[unsafe(no_mangle)]
pub extern "C" fn Py_IsInitialized() -> c_int {
    MAIN_INTERP.get().is_some() as c_int
}

#[unsafe(no_mangle)]
pub extern "C" fn Py_Initialize() {
    Py_InitializeEx(0);
}

#[unsafe(no_mangle)]
pub extern "C" fn Py_InitializeEx(_initsigs: c_int) {
    MAIN_INTERP.get_or_init(|| Interpreter::with_init(Default::default(), |_vm| {}).into());

    attach_vm_to_thread();
}

#[unsafe(no_mangle)]
pub extern "C" fn Py_Finalize() {
    let _ = Py_FinalizeEx();
}

#[unsafe(no_mangle)]
pub extern "C" fn Py_FinalizeEx() -> c_int {
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn Py_IsFinalizing() -> c_int {
    0
}
