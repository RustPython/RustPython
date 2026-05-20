use crate::get_main_interpreter;
use crate::pyerrors::init_exception_statics;
use crate::pystate::ensure_thread_has_vm_attached;
use core::ffi::{c_char, c_int};
use rustpython_vm::version::{MAJOR, MICRO, MINOR, VERSION_HEX};
use rustpython_vm::vm::thread::ThreadedVirtualMachine;
use rustpython_vm::{Context, Interpreter};
use std::ffi::c_ulong;
use std::sync::{LazyLock, Mutex};

pub(crate) static MAIN_INTERP: Mutex<Option<Interpreter>> = Mutex::new(None);

/// Request a thread local vm from the main interpreter
pub(crate) fn request_vm_from_interpreter() -> ThreadedVirtualMachine {
    get_main_interpreter()
        .as_ref()
        .expect("Interpreter not initialized")
        .enter(|vm| vm.new_thread())
}

#[unsafe(no_mangle)]
pub static Py_Version: c_ulong = VERSION_HEX as c_ulong;

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
        // Safety: Interpreter was not initialized before, so we can safely assume the statics are not used
        unsafe { init_exception_statics(&Context::genesis().exceptions) };
        *interp = Interpreter::with_init(Default::default(), |_vm| {}).into();
        drop(interp);
        ensure_thread_has_vm_attached();
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

#[unsafe(no_mangle)]
pub extern "C" fn Py_GetVersion() -> *const c_char {
    static VERSION: LazyLock<String> = LazyLock::new(|| format!("{MAJOR}.{MINOR}.{MICRO}"));
    VERSION.as_str().as_ptr() as *const c_char
}

#[cfg(test)]
mod tests {
    use pyo3::prelude::*;

    #[test]
    fn test_get_version() {
        Python::attach(|py| {
            let version = py.version_info();
            assert!(version >= (3, 14));
        });

        assert!(unsafe { pyo3::ffi::Py_Version } >= 0x030d0000);
    }
}
