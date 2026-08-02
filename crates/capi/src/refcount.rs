use crate::{PyObject, pystate::with_vm};
use core::ptr::NonNull;
use rustpython_vm::PyObjectRef;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _Py_DecRef(op: *mut PyObject) {
    // By dropping PyObjectRef, we will decrement the reference count.
    unsafe { drop(PyObjectRef::from_raw(NonNull::new_unchecked(op))) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _Py_IncRef(op: *mut PyObject) {
    // Don't drop the owned value, as we just want to increment the refcount.
    core::mem::forget(unsafe { (*op).to_owned() });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_NewRef(op: *mut PyObject) -> *mut PyObject {
    with_vm(|_vm| unsafe { (*op).to_owned() })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_REFCNT(op: *mut PyObject) -> isize {
    with_vm(|_vm| unsafe { &*op }.strong_count())
}

#[cfg(test)]
mod tests {
    use pyo3::ffi;
    use pyo3::prelude::*;
    use pyo3::types::PyList;

    #[test]
    fn refcount() {
        Python::attach(|py| unsafe {
            // A freshly created, non-empty list is uniquely owned here: its
            // reference count is private to this test (so parallel tests cannot
            // perturb it) and it is mortal (not interned), so incref then decref
            // must move the count by exactly one and back.
            let obj = PyList::new(py, [1, 2, 3]).unwrap();
            let ref_count = ffi::Py_REFCNT(obj.as_ptr());
            let obj_clone = obj.clone();
            assert_eq!(ffi::Py_REFCNT(obj.as_ptr()), ref_count + 1);
            drop(obj_clone);
            assert_eq!(ffi::Py_REFCNT(obj.as_ptr()), ref_count);
        });
    }
}
