use crate::PyObject;
use crate::pystate::with_vm;
use core::ffi::c_char;
use rustpython_vm::builtins::PyBytes;
use rustpython_vm::convert::IntoObject;

#[unsafe(no_mangle)]
pub extern "C" fn PyBytes_FromStringAndSize(bytes: *mut c_char, len: isize) -> *mut PyObject {
    with_vm(|vm| {
        if bytes.is_null() {
            std::ptr::null_mut()
        } else {
            let bytes_slice =
                unsafe { core::slice::from_raw_parts(bytes as *const u8, len as usize) };
            vm.ctx
                .new_bytes(bytes_slice.to_vec())
                .into_object()
                .into_raw()
                .as_ptr()
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyBytes_Size(bytes: *mut PyObject) -> isize {
    with_vm(|_vm| {
        let bytes = unsafe { &*bytes }
            .downcast_ref::<PyBytes>()
            .expect("PyBytes_Size argument must be a bytes object");
        bytes.as_bytes().len() as _
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyBytes_AsString(bytes: *mut PyObject) -> *mut c_char {
    with_vm(|_vm| {
        let bytes = unsafe { &*bytes }
            .downcast_ref::<PyBytes>()
            .expect("PyBytes_AsString argument must be a bytes object");
        bytes.as_bytes().as_ptr() as _
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
