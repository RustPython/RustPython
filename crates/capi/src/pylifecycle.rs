use crate::get_main_interpreter;
use crate::pystate::attach_vm_to_thread;
use core::ffi::c_int;
use rustpython_vm::Interpreter;
use rustpython_vm::vm::thread::ThreadedVirtualMachine;
use std::sync::Mutex;

pub(crate) static MAIN_INTERP: Mutex<Option<Interpreter>> = Mutex::new(None);

/// Request a thread local vm from the main interpreter
pub(crate) fn request_vm_from_interpreter() -> ThreadedVirtualMachine {
    get_main_interpreter()
        .as_ref()
        .expect("Interpreter not initialized")
        .enter(|vm| vm.new_thread())
}

#[unsafe(no_mangle)]
pub extern "C" fn Py_IsInitialized() -> c_int {
    get_main_interpreter().is_some() as c_int
}

#[unsafe(no_mangle)]
pub extern "C" fn Py_Initialize() {
    Py_InitializeEx(0);
}

#[unsafe(no_mangle)]
pub extern "C" fn Py_InitializeEx(_initsigs: c_int) {
    let mut interp = get_main_interpreter();
    if interp.is_none() {
        *interp = Interpreter::with_init(Default::default(), |_vm| {}).into();
        drop(interp);
        attach_vm_to_thread();
    }
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
