use core::ffi::c_char;
use core::ptr;

use crate::PyObject;

#[unsafe(no_mangle)]
pub extern "C" fn PyBytes_Size(_bytes: *mut PyObject) -> isize {
    crate::log_stub("PyBytes_Size");
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn PyBytes_AsString(_bytes: *mut PyObject) -> *mut c_char {
    crate::log_stub("PyBytes_AsString");
    ptr::null_mut()
}
