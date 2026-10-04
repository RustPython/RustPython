use crate::object::define_py_check;
use crate::util::FfiPtrExt;
use crate::{PyObject, pystate::with_vm};
use core::ffi::{CStr, c_char, c_int};
use rustpython_vm::builtins::PyBytes;

define_py_check!(fn PyBytes_Check, types.bytes_type);
define_py_check!(exact fn PyBytes_CheckExact, types.bytes_type);

#[unsafe(no_mangle)]
#[allow(clippy::uninit_vec)]
pub unsafe extern "C" fn PyBytes_FromStringAndSize(
    bytes: *mut c_char,
    len: isize,
) -> *mut PyObject {
    with_vm(|vm| {
        let len = len.try_into().map_err(|_| {
            vm.new_system_error("Negative size passed to PyBytes_FromStringAndSize")
        })?;

        let data = if bytes.is_null() {
            let mut data = Vec::with_capacity(len);
            unsafe { data.set_len(len) };
            data
        } else {
            unsafe { core::slice::from_raw_parts(bytes as *const u8, len) }.to_vec()
        };

        Ok(vm.ctx.new_bytes(data))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBytes_FromString(s: *const c_char) -> *mut PyObject {
    with_vm(|vm| {
        let data = unsafe { CStr::from_ptr(s) }.to_bytes().to_vec();
        Ok(vm.ctx.new_bytes(data))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBytes_FromObject(obj: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let obj = unsafe { obj.assume_borrowed() };
        if let Some(bytes) = obj.downcast_ref::<PyBytes>() {
            Ok(bytes.to_owned())
        } else {
            obj.try_bytes_like(vm, |bytes| vm.ctx.new_bytes(bytes.to_vec()))
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBytes_Size(bytes: *mut PyObject) -> isize {
    with_vm(|vm| {
        let bytes = unsafe { bytes.assume_borrowed_and_cast::<PyBytes>(vm) }?;
        Ok(bytes.as_bytes().len())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBytes_AsString(bytes: *mut PyObject) -> *mut c_char {
    with_vm(|vm| {
        let bytes = unsafe { bytes.assume_borrowed_and_cast::<PyBytes>(vm) }?;
        Ok(bytes.as_bytes().as_ptr())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBytes_AsStringAndSize(
    obj: *mut PyObject,
    buffer: *mut *mut c_char,
    length: *mut isize,
) -> c_int {
    with_vm(|vm| {
        let data = unsafe { obj.assume_borrowed_and_cast::<PyBytes>(vm)? }.as_bytes();

        if length.is_null() {
            if data.contains(&0) {
                return Err(vm.new_value_error("embedded null byte"));
            }
        } else {
            unsafe { *length = data.len() as isize };
        };

        unsafe { *buffer = data.as_ptr().cast_mut().cast() };

        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use pyo3::prelude::*;
    use pyo3::types::PyBytes;

    #[test]
    fn bytes() {
        Python::attach(|py| {
            let bytes = PyBytes::new(py, b"Hello, World!");
            assert_eq!(bytes.as_bytes(), b"Hello, World!");
        })
    }

    #[test]
    fn bytes_uninit() {
        Python::attach(|py| {
            let bytes = PyBytes::new_with(py, 13, |data| {
                data.copy_from_slice(b"Hello, World!");
                Ok(())
            })
            .unwrap();
            assert_eq!(bytes.as_bytes(), b"Hello, World!");
        })
    }
}
