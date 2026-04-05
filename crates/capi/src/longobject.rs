use core::ffi::{c_long, c_longlong, c_ulong, c_ulonglong};
use core::ptr;

use crate::PyObject;
use crate::pylifecycle::INTERP;
use rustpython_vm::PyObjectRef;
use rustpython_vm::builtins::PyInt;

#[unsafe(no_mangle)]
pub extern "C" fn PyLong_FromLong(value: c_long) -> *mut PyObject {
    INTERP.with(|interp_ref| {
        let interp = interp_ref.borrow();
        let interp = interp
            .as_ref()
            .expect("PyLong_FromLong called before Py_InitializeEx");

        interp.enter(|vm| {
            let obj: PyObjectRef = vm.ctx.new_int(value).into();
            obj.into_raw().as_ptr()
        })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyLong_FromLongLong(_value: c_longlong) -> *mut PyObject {
    crate::log_stub("PyLong_FromLongLong");
    ptr::null_mut()
}

#[unsafe(no_mangle)]
pub extern "C" fn PyLong_FromSsize_t(_value: isize) -> *mut PyObject {
    crate::log_stub("PyLong_FromSsize_t");
    ptr::null_mut()
}

#[unsafe(no_mangle)]
pub extern "C" fn PyLong_FromSize_t(_value: usize) -> *mut PyObject {
    crate::log_stub("PyLong_FromSize_t");
    ptr::null_mut()
}

#[unsafe(no_mangle)]
pub extern "C" fn PyLong_FromUnsignedLong(_value: c_ulong) -> *mut PyObject {
    crate::log_stub("PyLong_FromUnsignedLong");
    ptr::null_mut()
}

#[unsafe(no_mangle)]
pub extern "C" fn PyLong_FromUnsignedLongLong(_value: c_ulonglong) -> *mut PyObject {
    crate::log_stub("PyLong_FromUnsignedLongLong");
    ptr::null_mut()
}

#[unsafe(no_mangle)]
pub extern "C" fn PyLong_AsLong(obj: *mut PyObject) -> c_long {
    if obj.is_null() {
        panic!("PyLong_AsLong called with null object");
    }

    INTERP.with(|interp_ref| {
        let interp = interp_ref.borrow();
        let interp = interp
            .as_ref()
            .expect("PyLong_AsLong called before Py_InitializeEx");

        interp.enter(|_vm| {
            // SAFETY: non-null checked above; caller promises a valid PyObject pointer.
            let obj_ref = unsafe { &*obj };
            let int_obj = obj_ref
                .downcast_ref::<PyInt>()
                .expect("PyLong_AsLong currently only accepts int instances");

            int_obj
                .as_bigint()
                .try_into()
                .expect("PyLong_AsLong: value out of range for c_long")
        })
    })
}
