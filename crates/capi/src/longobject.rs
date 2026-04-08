use core::ffi::{c_long, c_longlong, c_ulong, c_ulonglong};

use crate::PyObject;
use crate::pyerrors::{PyErr_SetString, PyExc_OverflowError};
use crate::pystate::with_vm;
use rustpython_vm::builtins::PyInt;
use rustpython_vm::convert::IntoObject;

#[unsafe(no_mangle)]
pub extern "C" fn PyLong_FromLong(value: c_long) -> *mut PyObject {
    with_vm(|vm| vm.ctx.new_int(value).into_object().into_raw().as_ptr())
}

#[unsafe(no_mangle)]
pub extern "C" fn PyLong_FromLongLong(value: c_longlong) -> *mut PyObject {
    with_vm(|vm| vm.ctx.new_int(value).into_object().into_raw().as_ptr())
}

#[unsafe(no_mangle)]
pub extern "C" fn PyLong_FromSsize_t(value: isize) -> *mut PyObject {
    with_vm(|vm| vm.ctx.new_int(value).into_object().into_raw().as_ptr())
}

#[unsafe(no_mangle)]
pub extern "C" fn PyLong_FromSize_t(value: usize) -> *mut PyObject {
    with_vm(|vm| vm.ctx.new_int(value).into_object().into_raw().as_ptr())
}

#[unsafe(no_mangle)]
pub extern "C" fn PyLong_FromUnsignedLong(value: c_ulong) -> *mut PyObject {
    with_vm(|vm| vm.ctx.new_int(value).into_object().into_raw().as_ptr())
}

#[unsafe(no_mangle)]
pub extern "C" fn PyLong_FromUnsignedLongLong(value: c_ulonglong) -> *mut PyObject {
    with_vm(|vm| vm.ctx.new_int(value).into_object().into_raw().as_ptr())
}

#[unsafe(no_mangle)]
pub extern "C" fn PyLong_AsLong(obj: *mut PyObject) -> c_long {
    if obj.is_null() {
        panic!("PyLong_AsLong called with null object");
    }

    with_vm(|_vm| {
        // SAFETY: non-null checked above; caller promises a valid PyObject pointer.
        let obj_ref = unsafe { &*obj };
        let int_obj = obj_ref
            .downcast_ref::<PyInt>()
            .expect("PyLong_AsLong currently only accepts int instances");

        int_obj.as_bigint().try_into().unwrap_or_else(|_| unsafe {
            PyErr_SetString(
                PyExc_OverflowError.assume_init(),
                c"Python int too large to convert to C long".as_ptr(),
            );
            -1
        })
    })
}

#[cfg(test)]
mod tests {
    use pyo3::prelude::*;
    use pyo3::types::PyInt;

    #[test]
    fn test_py_int() {
        Python::attach(|py| {
            let number = PyInt::new(py, 123);
            assert!(number.is_instance_of::<PyInt>());
            assert_eq!(number.extract::<i32>().unwrap(), 123);
        })
    }
}
