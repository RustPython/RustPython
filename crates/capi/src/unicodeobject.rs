use crate::{PyObject, with_vm};
use core::ffi::{CStr, c_char, c_int};
use core::ptr::NonNull;
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

    with_vm(|vm| vm.ctx.new_str(text))
}

#[unsafe(no_mangle)]
pub extern "C" fn PyUnicode_AsUTF8AndSize(obj: *mut PyObject, size: *mut isize) -> *const c_char {
    with_vm(|vm| {
        let obj = unsafe {
            obj.as_ref()
                .expect("PyUnicode_AsUTF8AndSize called with null pointer")
        };

        let unicode = obj.try_downcast_ref::<PyStr>(vm)?;

        let str = unicode
            .to_str()
            .expect("only utf8 or ascii is currently supported in PyUnicode_AsUTF8AndSize");

        if !size.is_null() {
            unsafe { *size = str.len() as isize };
        }
        Ok(str.as_ptr())
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyUnicode_AsEncodedString(
    unicode: *mut PyObject,
    encoding: *const c_char,
    errors: *const c_char,
) -> *mut PyObject {
    with_vm(|vm| {
        let unicode = unsafe { &*unicode }
            .try_downcast_ref::<PyStr>(vm)?
            .to_owned();
        let encoding = if encoding.is_null() {
            "utf-8"
        } else {
            unsafe { CStr::from_ptr(encoding) }
                .to_str()
                .expect("encoding must be valid UTF-8")
        };
        let errors = if errors.is_null() {
            None
        } else {
            let errors = unsafe { CStr::from_ptr(errors) }
                .to_str()
                .expect("errors must be valid UTF-8");
            Some(vm.ctx.new_utf8_str(errors))
        };
        vm.state
            .codec_registry
            .encode_text(unicode, encoding, errors, vm)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyUnicode_InternInPlace(string: *mut *mut PyObject) {
    with_vm(|vm| {
        let old_str = unsafe { PyObjectRef::from_raw(NonNull::new_unchecked(*string)) }
            .downcast_exact::<PyStr>(vm)
            .expect("PyUnicode_InternInPlace called with non-string object");

        let interned: PyObjectRef = vm.ctx.intern_str(old_str).to_owned().into();

        unsafe { *string = interned.into_raw().as_ptr() }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyUnicode_EqualToUTF8AndSize(
    unicode: *mut PyObject,
    string: *const c_char,
    size: isize,
) -> c_int {
    with_vm(|vm| {
        let unicode = unsafe { &*unicode }.try_downcast_ref::<PyStr>(vm)?;
        let result = unsafe {
            let slice = slice::from_raw_parts(string as _, size as _);
            str::from_utf8(slice)
        }
        .ok()
        .and_then(|other| Some(unicode.to_str()? == other))
        .unwrap_or(false);

        Ok(result)
    })
}

#[cfg(test)]
mod tests {
    use pyo3::intern;
    use pyo3::prelude::*;
    use pyo3::types::PyString;

    #[test]
    fn test_unicode() {
        Python::attach(|py| {
            let string = PyString::new(py, "Hello, World!");
            assert!(string.is_instance_of::<PyString>());
            assert_eq!(string.to_str().unwrap(), "Hello, World!");
            assert_eq!(string, "Hello, World!");
        })
    }

    #[test]
    fn test_intern_str() {
        Python::attach(|py| {
            let _string = intern!(py, "Hello, World!");
        })
    }
}
