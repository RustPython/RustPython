use crate::{PyObject, with_vm};
use core::ffi::c_char;
use rustpython_vm::builtins::PyBytes;

#[unsafe(no_mangle)]
pub extern "C" fn PyBytes_FromStringAndSize(bytes: *mut c_char, len: isize) -> *mut PyObject {
    with_vm(|vm| {
        if bytes.is_null() {
            vm.ctx.new_bytes(vec![0; len as usize])
        } else {
            let bytes_slice =
                unsafe { core::slice::from_raw_parts(bytes as *const u8, len as usize) };
            vm.ctx.new_bytes(bytes_slice.to_vec())
        }
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
}
