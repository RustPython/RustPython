use core::ffi::c_int;

use crate::PyObject;

#[unsafe(no_mangle)]
pub extern "C" fn PyTraceBack_Print(_tb: *mut PyObject, _file: *mut PyObject) -> c_int {
    0
}
