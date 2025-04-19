// https://docs.python.org/3/c-api/float.html

use std::ffi;

use rustpython_vm::{PyObject, PyObjectRef};

/// Returns null if the string is not a valid float.
#[unsafe(export_name = "PyFloat_FromString")]
pub unsafe extern "C" fn float_from_string(value: *const ffi::c_char) -> *mut PyObject {
    let vm = crate::get_vm();
    let value_str = unsafe { std::ffi::CStr::from_ptr(value).to_str().unwrap() };
    match value_str.parse::<f64>() {
        Ok(value) => Into::<PyObjectRef>::into(vm.ctx.new_float(value))
            .into_raw()
            .as_ptr(),
        Err(_) => std::ptr::null_mut(),
    }
}

#[unsafe(export_name = "PyFloat_FromDouble")]
pub unsafe extern "C" fn float_from_double(value: ffi::c_double) -> *mut PyObject {
    let vm = crate::get_vm();
    Into::<PyObjectRef>::into(vm.ctx.new_float(value))
        .into_raw()
        .as_ptr()
}
