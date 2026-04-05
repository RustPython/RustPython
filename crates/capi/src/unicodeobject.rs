use core::ffi::c_char;
use core::ptr;
use core::slice;
use core::str;

use crate::PyObject;
use crate::pystate::with_vm;
use rustpython_vm::PyObjectRef;

#[unsafe(no_mangle)]
pub extern "C" fn PyUnicode_FromStringAndSize(s: *const c_char, len: isize) -> *mut PyObject {
    let len = usize::try_from(len).expect("PyUnicode_FromStringAndSize called with negative len");
    let text = if s.is_null() {
        if len != 0 {
            panic!("PyUnicode_FromStringAndSize called with null data and non-zero len");
        }
        ""
    } else {
        // SAFETY: caller passes a valid C buffer of length `len`.
        let bytes = unsafe { slice::from_raw_parts(s.cast::<u8>(), len) };
        str::from_utf8(bytes).expect("PyUnicode_FromStringAndSize got non-UTF8 data")
    };

    with_vm(|vm| {
        let obj: PyObjectRef = vm.ctx.new_str(text).into();
        obj.into_raw().as_ptr()
    })
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
