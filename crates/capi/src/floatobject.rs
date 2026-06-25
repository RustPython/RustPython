use crate::object::define_py_check;
use crate::{PyObject, pystate::with_vm};
use core::ffi::c_double;
use core::ptr::NonNull;
use rustpython_vm::AsObject;
use rustpython_vm::builtins::PyFloat;

define_py_check!(fn PyFloat_Check, types.float_type);
define_py_check!(exact fn PyFloat_CheckExact, types.float_type);

#[unsafe(no_mangle)]
pub extern "C" fn PyFloat_FromDouble(value: c_double) -> *mut PyObject {
    with_vm(|vm| vm.ctx.new_float(value))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyFloat_AsDouble(obj: *mut PyObject) -> c_double {
    with_vm(|vm| {
        let obj_ref = unsafe { &*obj };
        let float_obj = obj_ref
            .to_owned()
            .try_downcast::<PyFloat>(vm)
            .or_else(|_| obj_ref.try_float(vm))?;

        Ok(float_obj.to_f64())
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyFloat_GetMax() -> c_double {
    c_double::MAX
}

#[unsafe(no_mangle)]
pub extern "C" fn PyFloat_GetMin() -> c_double {
    c_double::MIN_POSITIVE
}

#[unsafe(no_mangle)]
pub extern "C" fn PyFloat_GetInfo() -> *mut PyObject {
    with_vm(|vm| {
        vm.sys_module
            .as_object()
            .get_attr("float_info", vm)
            .map(|obj| obj.into_raw().as_ptr())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyFloat_FromString(obj: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let obj = NonNull::new(obj)
            .ok_or_else(|| vm.new_type_error("float() argument must be a string or a number"))?;
        let obj = unsafe { obj.as_ref() }.to_owned();
        let float = rustpython_vm::builtins::parse_float_from_string(obj, vm)?;
        Ok(vm.ctx.new_float(float))
    })
}

#[cfg(false)]
mod tests {
    use core::f64::consts::PI;
    use pyo3::prelude::*;
    use pyo3::types::PyFloat;

    #[test]
    fn test_py_float() {
        Python::attach(|py| {
            let pi = PyFloat::new(py, PI);
            assert!(pi.is_instance_of::<PyFloat>());
            assert_eq!(pi.extract::<f64>().unwrap(), PI);
        })
    }
}
