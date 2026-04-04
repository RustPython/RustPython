use core::ffi::c_char;
use core::ptr;

use crate::PyObject;

#[unsafe(no_mangle)]
pub extern "C" fn PyUnicode_FromStringAndSize(_s: *const c_char, _len: isize) -> *mut PyObject {
    crate::log_stub("PyUnicode_FromStringAndSize");
    ptr::null_mut()
}

#[unsafe(no_mangle)]
pub extern "C" fn PyUnicode_AsUTF8AndSize(
    _unicode: *mut PyObject,
    _size: *mut isize,
) -> *const c_char {
    crate::log_stub("PyUnicode_AsUTF8AndSize");
    ptr::null()
}

#[unsafe(no_mangle)]
pub extern "C" fn PyUnicode_AsEncodedString(
    _unicode: *mut PyObject,
    _encoding: *const c_char,
    _errors: *const c_char,
) -> *mut PyObject {
    crate::log_stub("PyUnicode_AsEncodedString");
    ptr::null_mut()
}

#[unsafe(no_mangle)]
pub extern "C" fn PyUnicode_InternInPlace(_string: *mut *mut PyObject) {
    crate::log_stub("PyUnicode_InternInPlace");
}
