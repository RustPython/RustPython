use crate::PyObject;
use crate::object::define_py_check;
use crate::pystate::with_vm;
use core::ffi::c_char;
use rustpython_vm::builtins::PyByteArray;
use rustpython_vm::byte::bytes_from_object;

define_py_check!(fn PyByteArray_Check, types.bytearray_type);

/// # Safety
///
/// If `bytes` is `NULL`, the returned bytearray may contain uninitialized
/// bytes. The caller is responsible for initializing all bytes before any read.
#[unsafe(no_mangle)]
#[allow(clippy::uninit_vec)]
pub unsafe extern "C" fn PyByteArray_FromStringAndSize(
    bytes: *const c_char,
    len: isize,
) -> *mut PyObject {
    with_vm(|vm| {
        let len: usize = len.try_into().map_err(|_| {
            vm.new_system_error("Negative size passed to PyByteArray_FromStringAndSize")
        })?;

        let data = if bytes.is_null() {
            let mut data = Vec::with_capacity(len);
            // SAFETY: `bytes == NULL` follows CPython semantics here; caller must
            // initialize all bytes before any read. We keep this behavior for C-API
            // compatibility and to avoid unnecessary zero-initialization overhead.
            unsafe { data.set_len(len) };
            data
        } else {
            unsafe { core::slice::from_raw_parts(bytes.cast::<u8>(), len) }.to_vec()
        };

        Ok(vm.ctx.new_bytearray(data))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyByteArray_FromObject(obj: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let obj = unsafe { &*obj };
        let data = bytes_from_object(vm, obj)?;
        Ok(vm.ctx.new_bytearray(data))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyByteArray_Size(bytearray: *mut PyObject) -> isize {
    with_vm(|vm| {
        let bytearray = unsafe { &*bytearray }.try_downcast_ref::<PyByteArray>(vm)?;
        Ok(bytearray.borrow_buf().len())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyByteArray_AsString(bytearray: *mut PyObject) -> *mut c_char {
    with_vm(|vm| {
        let bytearray = unsafe { &*bytearray }.try_downcast_ref::<PyByteArray>(vm)?;
        Ok(bytearray.borrow_buf_mut().as_mut_ptr())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyByteArray_Resize(bytearray: *mut PyObject, len: isize) -> i32 {
    with_vm(|vm| {
        let bytearray = unsafe { &*bytearray }.try_downcast_ref::<PyByteArray>(vm)?;
        bytearray.resize(len, vm)?;
        Ok(())
    })
}

#[cfg(false)]
mod tests {
    use pyo3::prelude::*;
    use pyo3::types::{PyByteArray, PyBytes};

    #[test]
    fn bytearray_size() {
        Python::attach(|py| {
            let bytearray = PyByteArray::new(py, b"abc");
            assert_eq!(bytearray.len(), 3);
        })
    }

    #[test]
    fn bytearray_resize() {
        Python::attach(|py| {
            let bytearray = PyByteArray::new(py, b"abcde");
            bytearray.resize(3).unwrap();
            assert_eq!(bytearray.len(), 3);
            assert_eq!(bytearray.to_vec(), b"abc");
        })
    }

    #[test]
    fn bytearray_from_string_and_size() {
        Python::attach(|py| {
            let bytearray = PyByteArray::new(py, b"hello");
            assert_eq!(bytearray.len(), 5);
            assert_eq!(bytearray.to_vec(), b"hello");
        })
    }

    #[test]
    fn bytearray_new_with_zero_initialized() {
        Python::attach(|py| {
            let bytearray = PyByteArray::new_with(py, 4, |bytes| {
                bytes[..2].copy_from_slice(b"hi");
                Ok(())
            })
            .unwrap();
            assert_eq!(bytearray.len(), 4);
            assert_eq!(bytearray.to_vec(), b"hi\0\0");
        })
    }

    #[test]
    fn bytearray_from_object() {
        Python::attach(|py| {
            let source = PyBytes::new(py, b"ABC");
            let bytearray = PyByteArray::from(&source).unwrap();
            assert_eq!(bytearray.to_vec(), b"ABC");
        })
    }
}
