// https://docs.python.org/3/c-api/long.html

use std::ffi;

use rustpython_vm::{PyObject, PyObjectRef};

#[unsafe(export_name = "PyLong_FromLong")]
pub unsafe extern "C" fn long_from_long(value: ffi::c_long) -> *mut PyObject {
    let vm = crate::get_vm();
    Into::<PyObjectRef>::into(vm.ctx.new_int(value))
        .into_raw()
        .as_ptr()
}

#[unsafe(export_name = "PyLong_FromUnsignedLong")]
pub unsafe extern "C" fn long_from_unsigned_long(value: ffi::c_ulong) -> *mut PyObject {
    let vm = crate::get_vm();
    Into::<PyObjectRef>::into(vm.ctx.new_int(value))
        .into_raw()
        .as_ptr()
}

// TODO: PyLong_FromSsize_t
// TODO: PyLong_FromSize_t
#[unsafe(export_name = "PyLong_FromLongLong")]
pub unsafe extern "C" fn long_from_long_long(value: ffi::c_longlong) -> *mut PyObject {
    let vm = crate::get_vm();
    Into::<PyObjectRef>::into(vm.ctx.new_int(value))
        .into_raw()
        .as_ptr()
}

#[unsafe(export_name = "PyLong_FromUnsignedLongLong")]
pub unsafe extern "C" fn long_from_unsigned_long_long(value: ffi::c_ulonglong) -> *mut PyObject {
    let vm = crate::get_vm();
    Into::<PyObjectRef>::into(vm.ctx.new_int(value))
        .into_raw()
        .as_ptr()
}

#[unsafe(export_name = "PyLong_FromDouble")]
pub unsafe extern "C" fn long_from_double(value: ffi::c_double) -> *mut PyObject {
    let vm = crate::get_vm();
    let value = value as i64;
    Into::<PyObjectRef>::into(vm.ctx.new_int(value))
        .into_raw()
        .as_ptr()
}
