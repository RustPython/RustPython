use std::ptr::NonNull;
use rustpython_vm::PyObjectRef;
use crate::{PyObject};

#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn _Py_DecRef(op: *mut PyObject) {
    let Some(ptr) = NonNull::new(op) else {
        return;
    };

    let owned = unsafe {
        PyObjectRef::from_raw(ptr)
    };

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
    let owned = unsafe {
        (*op).to_owned()
    };

    std::mem::forget(owned);
}
