use crate::PyObject;
use crate::pystate::with_vm;
use core::ffi::c_char;
use core::ptr;
use core::slice;
use core::str;
use rustpython_vm::PyObjectRef;
use rustpython_vm::builtins::PyStr;

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
pub extern "C" fn PyUnicode_AsUTF8AndSize(obj: *mut PyObject, size: *mut isize) -> *const c_char {
    with_vm(|_vm| {
        let obj = unsafe {
            obj.as_ref()
                .expect("PyUnicode_AsUTF8AndSize called with null pointer")
        };

        let unicode = obj
            .downcast_ref::<PyStr>()
            .expect("PyUnicode_AsUTF8AndSize called with non-unicode object");

        let str = unicode
            .to_str()
            .expect("only utf8 or ascii is currently supported in PyUnicode_AsUTF8AndSize");

        if !size.is_null() {
            unsafe { *size = str.len() as isize };
        }
        str.as_ptr() as _
    })
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

#[cfg(test)]
mod tests {
    use pyo3::prelude::*;
    use pyo3::types::PyString;

    #[test]
    fn test_unicode() {
        Python::attach(|py| {
            let string = PyString::new(py, "Hello, World!");
            assert!(string.is_instance_of::<PyString>());
            assert_eq!(string.to_str().unwrap(), "Hello, World!");
        })
    }
}
