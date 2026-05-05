use crate::{PyObject, with_vm};
use core::ffi::c_char;
use rustpython_vm::builtins::PyBytes;

#[unsafe(no_mangle)]
pub extern "C" fn PyBytes_FromStringAndSize(bytes: *mut c_char, len: isize) -> *mut PyObject {
    with_vm(|vm| {
        let data = if bytes.is_null() {
            let mut data = Vec::with_capacity(len as usize);
            unsafe { data.set_len(len as usize) };
            data
        } else {
            unsafe { core::slice::from_raw_parts(bytes as *const u8, len as usize) }.to_vec()
        };
        vm.ctx.new_bytes(data)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyBytes_Size(bytes: *mut PyObject) -> isize {
    with_vm(|vm| {
        let bytes = unsafe { &*bytes }.try_downcast_ref::<PyBytes>(vm)?;
        Ok(bytes.as_bytes().len())
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyBytes_AsString(bytes: *mut PyObject) -> *mut c_char {
    with_vm(|vm| {
        let bytes = unsafe { &*bytes }.try_downcast_ref::<PyBytes>(vm)?;
        Ok(bytes.as_bytes().as_ptr())
    })
}

#[cfg(test)]
mod tests {
    use pyo3::prelude::*;
    use pyo3::types::PyBytes;

    #[test]
    fn test_bytes() {
        Python::attach(|py| {
            let bytes = PyBytes::new(py, b"Hello, World!");
            assert_eq!(bytes.as_bytes(), b"Hello, World!");
        })
    }

    #[test]
    fn test_bytes_uninit() {
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
