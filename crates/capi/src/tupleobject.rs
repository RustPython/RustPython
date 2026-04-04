use core::ptr;

use crate::PyObject;

#[unsafe(no_mangle)]
pub extern "C" fn PyTuple_Size(_tuple: *mut PyObject) -> isize {
    crate::log_stub("PyTuple_Size");
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn PyTuple_GetItem(_tuple: *mut PyObject, _pos: isize) -> *mut PyObject {
    crate::log_stub("PyTuple_GetItem");
    ptr::null_mut()
}
