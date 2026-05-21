use crate::object::define_py_check;
use crate::{PyObject, pystate::with_vm};
use core::ffi::{c_int, c_long};
use rustpython_vm::AsObject;

define_py_check!(fn PyBool_Check, types.bool_type);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_IsTrue(obj: *mut PyObject) -> c_int {
    with_vm(|vm| unsafe { obj.as_ref().is_some_and(|obj| obj.is(&vm.ctx.true_value)) })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_IsFalse(obj: *mut PyObject) -> c_int {
    with_vm(|vm| unsafe { obj.as_ref().is_some_and(|obj| obj.is(&vm.ctx.false_value)) })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyBool_FromLong(value: c_long) -> *mut PyObject {
    with_vm(|vm| {
        if value == 0 {
            &vm.ctx.false_value
        } else {
            &vm.ctx.true_value
        }
        .to_owned()
    })
}
