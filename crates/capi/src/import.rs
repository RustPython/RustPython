use core::ptr;

use crate::PyObject;

#[unsafe(no_mangle)]
pub extern "C" fn PyImport_Import(_name: *mut PyObject) -> *mut PyObject {
    crate::log_stub("PyImport_Import");
    ptr::null_mut()
}
