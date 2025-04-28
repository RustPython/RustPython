use std::{ffi, ptr};

use rustpython_vm::{PyObject, PyObjectRef};

#[unsafe(export_name = "PyErr_NewException")]
pub unsafe extern "C" fn err_new_exception(
    name: *const ffi::c_char,
    _base: *mut ffi::c_void,
    _dict: *mut ffi::c_void,
) -> *mut PyObject {
    let vm = crate::get_vm();
    let name_str = unsafe { std::ffi::CStr::from_ptr(name).to_str().unwrap() };
    let last_dot = match name_str.rfind('.') {
        Some(x) => x,
        None => {
            // TODO: Set error
            return ptr::null_mut();
        }
    };
    let module = &name_str[..last_dot];
    let name = &name_str[last_dot + 1..];
    Into::<PyObjectRef>::into(vm.ctx.new_exception_type(
        module,
        name,
        Some(vec![vm.ctx.exceptions.exception_type.to_owned()]),
    ))
    .into_raw()
    .as_ptr()
}
