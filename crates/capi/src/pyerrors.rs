use core::ffi::{c_char, c_int};
use core::ptr;

use crate::PyObject;

#[unsafe(no_mangle)]
pub static mut PyExc_BaseException: *mut PyObject = ptr::null_mut();

#[unsafe(no_mangle)]
pub static mut PyExc_TypeError: *mut PyObject = ptr::null_mut();

#[unsafe(no_mangle)]
pub static mut PyExc_OverflowError: *mut PyObject = ptr::null_mut();

#[unsafe(no_mangle)]
pub extern "C" fn PyErr_GetRaisedException() -> *mut PyObject {
    crate::log_stub("PyErr_GetRaisedException");
    ptr::null_mut()
}

#[unsafe(no_mangle)]
pub extern "C" fn PyErr_SetRaisedException(_exc: *mut PyObject) {
    crate::log_stub("PyErr_SetRaisedException");
}

#[unsafe(no_mangle)]
pub extern "C" fn PyErr_SetObject(_exception: *mut PyObject, _value: *mut PyObject) {
    crate::log_stub("PyErr_SetObject");
}

#[unsafe(no_mangle)]
pub extern "C" fn PyErr_SetString(_exception: *mut PyObject, _message: *const c_char) {
    crate::log_stub("PyErr_SetString");
}

#[unsafe(no_mangle)]
pub extern "C" fn PyErr_PrintEx(_set_sys_last_vars: c_int) {
    crate::log_stub("PyErr_PrintEx");
}

#[unsafe(no_mangle)]
pub extern "C" fn PyErr_WriteUnraisable(_obj: *mut PyObject) {
    crate::log_stub("PyErr_WriteUnraisable");
}

#[unsafe(no_mangle)]
pub extern "C" fn PyErr_NewExceptionWithDoc(
    _name: *const c_char,
    _doc: *const c_char,
    _base: *mut PyObject,
    _dict: *mut PyObject,
) -> *mut PyObject {
    crate::log_stub("PyErr_NewExceptionWithDoc");
    ptr::null_mut()
}

#[unsafe(no_mangle)]
pub extern "C" fn PyException_GetTraceback(_exc: *mut PyObject) -> *mut PyObject {
    crate::log_stub("PyException_GetTraceback");
    ptr::null_mut()
}
