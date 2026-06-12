use crate::object::define_py_check;
use crate::{PyObject, pystate::with_vm};
use core::ffi::{CStr, c_char, c_int};
use core::ptr::NonNull;
use core::slice;
use core::str;
use rustpython_vm::PyObjectRef;
use rustpython_vm::builtins::PyStr;

define_py_check!(fn PyUnicode_Check, types.str_type);
define_py_check!(exact fn PyUnicode_CheckExact, types.str_type);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_FromStringAndSize(
    s: *const c_char,
    len: isize,
) -> *mut PyObject {
    with_vm(|vm| {
        let len: usize = len
            .try_into()
            .map_err(|_| vm.new_system_error("length must be non-negative"))?;

        let text = if s.is_null() {
            if len != 0 {
                return Err(vm.new_system_error(
                    "PyUnicode_FromStringAndSize called with null data and non-zero len",
                ));
            }
            ""
        } else {
            let bytes = unsafe { slice::from_raw_parts(s.cast::<u8>(), len) };
            str::from_utf8(bytes).expect("PyUnicode_FromStringAndSize got non-UTF8 data")
        };

        Ok(vm.ctx.new_str(text))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsUTF8AndSize(
    obj: *mut PyObject,
    size: *mut isize,
) -> *const c_char {
    with_vm(|vm| {
        let unicode = unsafe { &*obj }.try_downcast_ref::<PyStr>(vm)?;

        let str = unicode.to_str().ok_or_else(|| {
            vm.new_system_error("PyUnicode_AsUTF8AndSize only supports UTF-8 or ASCII strings")
        })?;

        if size.is_null() {
            // We do not support null size arguments because the returned string is not NULL terminated.
            return Err(
                vm.new_system_error("size argument to PyUnicode_AsUTF8AndSize cannot be null")
            );
        }

        unsafe { *size = str.len() as isize };
        Ok(str.as_ptr())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsEncodedString(
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
pub unsafe extern "C" fn PyUnicode_AsUTF8String(unicode: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let unicode = unsafe { &*unicode }
            .try_downcast_ref::<PyStr>(vm)?
            .to_owned();
        vm.state
            .codec_registry
            .encode_text(unicode, "utf-8", None, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeFSDefaultAndSize(
    s: *const c_char,
    size: isize,
) -> *mut PyObject {
    with_vm(|vm| {
        let size: usize = size
            .try_into()
            .map_err(|_| vm.new_system_error("size must be non-negative"))?;

        let bytes = if s.is_null() {
            if size != 0 {
                return Err(vm.new_system_error(
                    "PyUnicode_DecodeFSDefaultAndSize called with null data and non-zero size",
                ));
            }
            &[][..]
        } else {
            unsafe { slice::from_raw_parts(s.cast::<u8>(), size) }
        };

        vm.state.codec_registry.decode_text(
            vm.ctx.new_bytes(bytes.to_vec()).into(),
            vm.fs_encoding().as_str(),
            Some(vm.fs_encode_errors().to_owned()),
            vm,
        )
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_EncodeFSDefault(unicode: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let unicode = unsafe { &*unicode }
            .try_downcast_ref::<PyStr>(vm)?
            .to_owned();
        vm.state.codec_registry.encode_text(
            unicode,
            vm.fs_encoding().as_str(),
            Some(vm.fs_encode_errors().to_owned()),
            vm,
        )
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_FromEncodedObject(
    obj: *mut PyObject,
    encoding: *const c_char,
    errors: *const c_char,
) -> *mut PyObject {
    with_vm(|vm| {
        let obj = unsafe { &*obj };

        if obj.downcast_ref::<PyStr>().is_some() {
            return Err(vm.new_type_error("decoding str is not supported"));
        }

        let encoding = if encoding.is_null() {
            "utf-8"
        } else {
            unsafe { CStr::from_ptr(encoding) }
                .to_str()
                .map_err(|_| vm.new_system_error("encoding must be valid UTF-8"))?
        };
        let errors = if errors.is_null() {
            None
        } else {
            let errors = unsafe { CStr::from_ptr(errors) }
                .to_str()
                .map_err(|_| vm.new_system_error("errors must be valid UTF-8"))?;
            Some(vm.ctx.new_utf8_str(errors))
        };

        obj.try_bytes_like(vm, |b| {
            vm.state.codec_registry.decode_text(
                vm.ctx.new_bytes(b.to_vec()).into(),
                encoding,
                errors,
                vm,
            )
        })?
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_InternInPlace(string: *mut *mut PyObject) {
    with_vm(|vm| {
        let old_str = unsafe { PyObjectRef::from_raw(NonNull::new_unchecked(*string)) }
            .downcast_exact::<PyStr>(vm)
            .expect("PyUnicode_InternInPlace called with non-string object");

        let interned: PyObjectRef = vm.ctx.intern_str(old_str).to_owned().into();

        unsafe { *string = interned.into_raw().as_ptr() }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_EqualToUTF8AndSize(
    unicode: *mut PyObject,
    string: *const c_char,
    size: isize,
) -> c_int {
    with_vm(|vm| {
        let size = size.try_into().map_err(|_| {
            vm.new_system_error("Negative size passed to PyUnicode_EqualToUTF8AndSize")
        })?;

        let unicode = unsafe { &*unicode }.try_downcast_ref::<PyStr>(vm)?;
        let result = unsafe {
            let slice = slice::from_raw_parts(string as _, size);
            str::from_utf8(slice)
        }
        .ok()
        .and_then(|other| Some(unicode.to_str()? == other))
        .unwrap_or(false);

        Ok(result)
    })
}

#[cfg(false)]
mod tests {
    use std::ffi::{OsStr, OsString};

    use pyo3::intern;
    use pyo3::prelude::*;
    use pyo3::types::{PyBytes, PyString, PyStringMethods};

    #[cfg(unix)]
    use std::os::unix::ffi::OsStrExt;

    #[test]
    fn unicode() {
        Python::attach(|py| {
            let string = PyString::new(py, "Hello, World!");
            assert!(string.is_instance_of::<PyString>());
            assert_eq!(string.to_str().unwrap(), "Hello, World!");
            assert_eq!(string, "Hello, World!");
        })
    }

    #[test]
    fn intern_str() {
        Python::attach(|py| {
            let _string = intern!(py, "Hello, World!");
        })
    }

    #[test]
    fn encode_utf8_via_wrapper() {
        Python::attach(|py| {
            let s = PyString::new(py, "h\u{00E9}llo");
            let encoded = s.encode_utf8().unwrap();
            assert_eq!(encoded.as_bytes(), "h\u{00E9}llo".as_bytes());
        })
    }

    #[test]
    fn from_encoded_object_bytes() {
        Python::attach(|py| {
            let src = PyBytes::new(py, b"h\xC3\xA9llo");
            let s = PyString::from_encoded_object(src.as_any(), None, None).unwrap();
            assert_eq!(s.to_str().unwrap(), "h\u{00E9}llo");
        })
    }

    #[cfg(unix)]
    #[test]
    fn fs_default_roundtrip_non_utf8_unix() {
        Python::attach(|py| {
            let original = OsStr::from_bytes(&[b'f', b'o', 0x80]);
            let py_str = original.into_pyobject(py).unwrap();
            let roundtrip: OsString = py_str.extract().unwrap();
            assert_eq!(roundtrip.as_os_str().as_bytes(), original.as_bytes());
        })
    }

    #[test]
    fn fs_default_roundtrip_utf8() {
        Python::attach(|py| {
            let original = OsStr::new("hello.txt");
            let py_str = original.into_pyobject(py).unwrap();
            let roundtrip: OsString = py_str.extract().unwrap();
            assert_eq!(roundtrip, original);
        })
    }
}
