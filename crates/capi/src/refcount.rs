use crate::PyObject;
use core::ptr::NonNull;
use rustpython_vm::PyObjectRef;

#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn _Py_DecRef(op: *mut PyObject) {
    // By dropping PyObjectRef, we will decrement the reference count.
    unsafe { PyObjectRef::from_raw(NonNull::new_unchecked(op)) };
}

#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn _Py_IncRef(op: *mut PyObject) {
    // Don't drop the owned value, as we just want to increment the refcount.
    core::mem::forget(unsafe { (*op).to_owned() });
}
