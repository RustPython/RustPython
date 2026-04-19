use crate::{PyObject, with_vm};
use core::ffi::c_double;

#[unsafe(no_mangle)]
pub extern "C" fn PyFloat_FromDouble(value: c_double) -> *mut PyObject {
    with_vm(|vm| vm.ctx.new_float(value))
}
