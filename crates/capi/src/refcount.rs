use crate::{PyObject, with_vm};
use core::ptr::NonNull;
use rustpython_vm::PyObjectRef;

#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn _Py_DecRef(op: *mut PyObject) {
    let Some(ptr) = NonNull::new(op) else {
        return;
    };

    let owned = unsafe { PyObjectRef::from_raw(ptr) };

    // Dropping so we decrement the refcount
    drop(owned);
}

#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn _Py_IncRef(op: *mut PyObject) {
    if op.is_null() {
        return;
    }

    // SAFETY: op is non-null and expected to be a valid pointer for this shim.
    let owned = unsafe { (*op).to_owned() };

    core::mem::forget(owned);
}

#[unsafe(no_mangle)]
pub extern "C" fn Py_REFCNT(op: *mut PyObject) -> isize {
    with_vm(|_vm| unsafe { &*op }.strong_count() as isize)
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
