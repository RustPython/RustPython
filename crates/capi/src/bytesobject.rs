use crate::object::define_py_check;
use crate::util::FfiPtrExt;
use crate::{PyObject, pystate::with_vm};
use core::ffi::{CStr, c_char, c_int};
use rustpython_vm::builtins::PyBytes;
use rustpython_vm::convert::IntoObject;

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
        }

        unsafe { *buffer = data.as_ptr().cast_mut().cast() };

        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBytes_Concat(bytes: *mut *mut PyObject, newpart: *mut PyObject) {
    let _: () = with_vm(|vm| {
        let Some(left_obj) = (unsafe { (*bytes).assume_owned_or_opt() }) else {
            return Ok(());
        };
        unsafe { *bytes = core::ptr::null_mut() };
        let mut data = left_obj.try_bytes_like(vm, |b| b.to_vec())?;
        let Some(newpart_obj) = (unsafe { newpart.assume_borrowed_or_opt() }) else {
            return Ok(());
        };
        let right = newpart_obj.try_bytes_like(vm, |b| b.to_vec())?;
        data.extend_from_slice(&right);
        let res = vm.ctx.new_bytes(data);
        unsafe {
            *bytes = res.into_object().into_raw().as_ptr();
        }
        Ok(())
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBytes_ConcatAndDel(bytes: *mut *mut PyObject, newpart: *mut PyObject) {
    unsafe { PyBytes_Concat(bytes, newpart) };
    let _ = unsafe { newpart.assume_owned_or_opt() };
}

#[cfg(test)]
mod tests {
    use pyo3::prelude::*;
    use pyo3::types::{PyByteArray, PyBytes};

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

    #[test]
    fn bytes_concat() {
        Python::attach(|py| {
            let a = PyBytes::new(py, b"hello ");
            let b = PyBytes::new(py, b"world");
            unsafe { crate::refcount::Py_IncRef(a.as_ptr().cast()) };
            let mut ptr = a.as_ptr().cast();
            unsafe {
                super::PyBytes_Concat(&mut ptr, b.as_ptr().cast());
                let res = pyo3::Bound::from_owned_ptr(py, ptr.cast());
                let res_bytes = res.cast::<PyBytes>().unwrap();
                assert_eq!(res_bytes.as_bytes(), b"hello world");
            }

            // Test bytearray on left and bytes on right
            let ba = PyByteArray::new(py, b"hello ");
            unsafe { crate::refcount::Py_IncRef(ba.as_ptr().cast()) };
            let mut ptr2 = ba.as_ptr().cast();
            unsafe {
                super::PyBytes_Concat(&mut ptr2, b.as_ptr().cast());
                let res = pyo3::Bound::from_owned_ptr(py, ptr2.cast());
                let res_bytes = res.cast::<PyBytes>().unwrap();
                assert_eq!(res_bytes.as_bytes(), b"hello world");
            }

            // Test bytes on left and bytearray on right
            let a2 = PyBytes::new(py, b"hello ");
            unsafe { crate::refcount::Py_IncRef(a2.as_ptr().cast()) };
            let mut ptr3 = a2.as_ptr().cast();
            unsafe {
                super::PyBytes_Concat(&mut ptr3, ba.as_ptr().cast());
                let res = pyo3::Bound::from_owned_ptr(py, ptr3.cast());
                let res_bytes = res.cast::<PyBytes>().unwrap();
                assert_eq!(res_bytes.as_bytes(), b"hello hello ");
            }

            // Test PyBytes_ConcatAndDel
            let a3 = PyBytes::new(py, b"foo ");
            let b3 = PyBytes::new(py, b"bar");
            unsafe {
                crate::refcount::Py_IncRef(a3.as_ptr().cast());
                crate::refcount::Py_IncRef(b3.as_ptr().cast());
            }
            let mut ptr4 = a3.as_ptr().cast();
            unsafe {
                super::PyBytes_ConcatAndDel(&mut ptr4, b3.as_ptr().cast());
                let res = pyo3::Bound::from_owned_ptr(py, ptr4.cast());
                let res_bytes = res.cast::<PyBytes>().unwrap();
                assert_eq!(res_bytes.as_bytes(), b"foo bar");
            }

            // Test newpart == NULL clears *bytes
            let a4 = PyBytes::new(py, b"temp");
            unsafe { crate::refcount::Py_IncRef(a4.as_ptr().cast()) };
            let mut ptr5 = a4.as_ptr().cast();
            unsafe {
                super::PyBytes_Concat(&mut ptr5, core::ptr::null_mut());
                assert!(ptr5.is_null());
            }

            // Test *bytes == NULL is a no-op
            let mut ptr6: *mut crate::PyObject = core::ptr::null_mut();
            unsafe {
                super::PyBytes_Concat(&mut ptr6, b.as_ptr().cast());
                assert!(ptr6.is_null());
            }
        })
    }
}
