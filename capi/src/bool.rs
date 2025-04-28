// https://docs.python.org/3/c-api/bool.html

use std::ffi;

use rustpython_vm::{PyObject, PyObjectRef};

// TODO: Everything else

#[unsafe(export_name = "PyBool_FromLong")]
pub unsafe extern "C" fn bool_from_long(value: ffi::c_long) -> *mut PyObject {
    let vm = crate::get_vm();
    Into::<PyObjectRef>::into(vm.ctx.new_bool(value != 0))
        .into_raw()
        .as_ptr()
}
