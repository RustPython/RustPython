use core::ffi::{c_long, c_longlong, c_ulong, c_ulonglong};
use core::ptr;

use crate::PyObject;

#[unsafe(no_mangle)]
pub extern "C" fn PyLong_FromLong(_value: c_long) -> *mut PyObject {
    crate::log_stub("PyLong_FromLong");
    ptr::null_mut()
}

#[unsafe(no_mangle)]
pub extern "C" fn PyLong_FromLongLong(_value: c_longlong) -> *mut PyObject {
    crate::log_stub("PyLong_FromLongLong");
    ptr::null_mut()
}

#[unsafe(no_mangle)]
pub extern "C" fn PyLong_FromSsize_t(_value: isize) -> *mut PyObject {
    crate::log_stub("PyLong_FromSsize_t");
    ptr::null_mut()
}

#[unsafe(no_mangle)]
pub extern "C" fn PyLong_FromSize_t(_value: usize) -> *mut PyObject {
    crate::log_stub("PyLong_FromSize_t");
    ptr::null_mut()
}

#[unsafe(no_mangle)]
pub extern "C" fn PyLong_FromUnsignedLong(_value: c_ulong) -> *mut PyObject {
    crate::log_stub("PyLong_FromUnsignedLong");
    ptr::null_mut()
}

#[unsafe(no_mangle)]
pub extern "C" fn PyLong_FromUnsignedLongLong(_value: c_ulonglong) -> *mut PyObject {
    crate::log_stub("PyLong_FromUnsignedLongLong");
    ptr::null_mut()
}
