use crate::log_stub;
use core::ffi::c_int;
use core::ptr;

#[allow(non_camel_case_types)]
type PyGILState_STATE = c_int;

#[repr(C)]
pub struct PyThreadState {
    _interp: *mut std::ffi::c_void,
}

#[unsafe(no_mangle)]
pub extern "C" fn PyGILState_Ensure() -> PyGILState_STATE {
    log_stub("PyGILState_Ensure");
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn PyGILState_Release(_state: PyGILState_STATE) {
    log_stub("PyGILState_Release");
}

#[unsafe(no_mangle)]
pub extern "C" fn PyEval_SaveThread() -> *mut PyThreadState {
    log_stub("PyEval_SaveThread");
    ptr::null_mut()
}

#[unsafe(no_mangle)]
pub extern "C" fn PyEval_RestoreThread(_tstate: *mut PyThreadState) {
    log_stub("PyEval_RestoreThread");
}
