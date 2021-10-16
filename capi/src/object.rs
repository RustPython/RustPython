#![allow(non_snake_case)]
//! https://docs.python.org/3/c-api/object.html

use crate::vm::{PyObjectPtr, PyObjectRef, PyResult, _with_current_thread_vm};

fn unwrap_pyresult(r: PyResult) -> PyObjectRef {
    // TODO: set PyErr
    r.expect("TODO: PyErr handling")
}

fn into_raw(o: PyObjectRef) -> *const PyObjectPtr {
    PyObjectRef::into_raw(o) as *const _
}

#[no_mangle]
pub unsafe extern "C" fn PyObject_Repr(o: PyObjectPtr) -> *const PyObjectPtr {
    into_raw(unwrap_pyresult(
        _with_current_thread_vm(&*o, |vm| o.repr(vm))
            .expect("TODO: handle vm error")
            .map(Into::into),
    ))
}
