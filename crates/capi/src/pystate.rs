use crate::pylifecycle::{INITIALIZED, Py_InitializeEx};
use crate::util::FfiResult;
use core::ffi::c_int;
use core::ptr;
use core::sync::atomic::Ordering;
use rustpython_vm::VirtualMachine;
use rustpython_vm::vm::thread::try_with_current_vm;

pub(crate) fn with_vm<R: FfiResult<O>, O>(f: impl FnOnce(&VirtualMachine) -> R) -> O {
    if !INITIALIZED.load(Ordering::Acquire) {
        Py_InitializeEx(0);
    }
    try_with_current_vm(|vm| f(vm).into_output(vm))
        .unwrap_or_else(|| panic!("rustpython-capi called without an active RustPython VM"))
}

#[allow(non_camel_case_types)]
type PyGILState_STATE = c_int;

#[repr(C)]
pub struct PyThreadState {
    _interp: *mut core::ffi::c_void,
}

#[unsafe(no_mangle)]
pub extern "C" fn PyGILState_Ensure() -> PyGILState_STATE {
    if !INITIALIZED.load(Ordering::Acquire) {
        Py_InitializeEx(0);
    }
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn PyGILState_Release(_state: PyGILState_STATE) {}

#[unsafe(no_mangle)]
pub extern "C" fn PyEval_SaveThread() -> *mut PyThreadState {
    ptr::null_mut()
}

#[unsafe(no_mangle)]
pub extern "C" fn PyEval_RestoreThread(_tstate: *mut PyThreadState) {}
