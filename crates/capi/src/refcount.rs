use crate::handles::{decref_wrapper, incref_wrapper, resolve_object_handle, wrapper_refcnt};
use crate::{PyObject, with_vm};
use core::ptr::NonNull;
use rustpython_vm::PyObjectRef;

#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn _Py_DecRef(op: *mut PyObject) {
    if unsafe { decref_wrapper(op) } {
        return;
    }
    // By dropping PyObjectRef, we will decrement the reference count.
    unsafe { PyObjectRef::from_raw(NonNull::new_unchecked(resolve_object_handle(op))) };
}

#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn _Py_IncRef(op: *mut PyObject) {
    if unsafe { incref_wrapper(op) } {
        return;
    }
    // Don't drop the owned value, as we just want to increment the refcount.
    core::mem::forget(unsafe { (*resolve_object_handle(op)).to_owned() });
}

#[unsafe(no_mangle)]
pub extern "C" fn Py_DecRef(op: *mut PyObject) {
    _Py_DecRef(op);
}

#[unsafe(no_mangle)]
pub extern "C" fn Py_IncRef(op: *mut PyObject) {
    _Py_IncRef(op);
}

#[unsafe(no_mangle)]
pub extern "C" fn Py_REFCNT(op: *mut PyObject) -> isize {
    if let Some(refcnt) = unsafe { wrapper_refcnt(op) } {
        return refcnt;
    }
    with_vm(|_vm| unsafe { &*resolve_object_handle(op) }.strong_count())
}

#[cfg(test)]
mod tests {
    use pyo3::prelude::*;
    use pyo3::types::PyInt;
    use pyo3::{PyTypeInfo, ffi};

    #[test]
    fn test_refcount() {
        Python::attach(|py| unsafe {
            let obj = PyInt::type_object(py);
            let ref_count = ffi::Py_REFCNT(obj.as_ptr());
            let obj_clone = obj.clone();
            assert_eq!(ffi::Py_REFCNT(obj.as_ptr()), ref_count + 1);
            drop(obj_clone);
            assert_eq!(ffi::Py_REFCNT(obj.as_ptr()), ref_count);
        });
    }
}
