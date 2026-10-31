use crate::PyObject;
use crate::object::define_py_check;
use crate::pystate::with_vm;
use crate::util::{CStrExt, FfiPtrExt};
use rustpython_vm::AsObject;
use rustpython_vm::builtins::{PyModule, PyStr};

define_py_check!(fn PyModule_Check, types.module_type);
define_py_check!(exact fn PyModule_CheckExact, types.module_type);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyModule_GetNameObject(module: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let module = unsafe { module.assume_borrowed_and_cast::<PyModule>(vm) }?;
        let dict = module.dict();
        let name = dict
            .get_item_opt(rustpython_vm::identifier!(vm, __name__), vm)?
            .and_then(|obj| obj.downcast_ref::<PyStr>().map(ToOwned::to_owned));
        name.ok_or_else(|| vm.new_system_error("nameless module"))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyModule_GetFilenameObject(module: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let module = unsafe { module.assume_borrowed_and_cast::<PyModule>(vm) }?;
        let dict = module.dict();
        let filename = dict
            .get_item_opt(rustpython_vm::identifier!(vm, __file__), vm)?
            .and_then(|obj| obj.downcast_ref::<PyStr>().map(ToOwned::to_owned));
        filename.ok_or_else(|| vm.new_system_error("module filename missing"))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyModule_NewObject(name: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| -> rustpython_vm::PyResult<_> {
        let name = unsafe { name.assume_borrowed_and_cast::<PyStr>(vm) }?;
        let name = name
            .to_str()
            .ok_or_else(|| vm.new_system_error("module name must be valid UTF-8"))?;
        Ok(vm.new_module(name, vm.ctx.new_dict(), None))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyModule_New(name: *const core::ffi::c_char) -> *mut PyObject {
    with_vm(|vm| {
        let name_str = unsafe { name.try_as_str(vm) }?;
        Ok(vm.new_module(name_str, vm.ctx.new_dict(), None))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyModule_GetDict(module: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let module = unsafe { module.assume_borrowed_and_cast::<PyModule>(vm) }?;
        Ok(module.dict().as_object().as_raw().cast_mut())
    })
}

#[cfg(test)]
mod tests {
    use pyo3::prelude::*;
    use pyo3::types::{PyAnyMethods, PyDict, PyModule, PyString};

    #[test]
    fn module_new_and_get_dict() {
        Python::attach(|py| {
            let mod_name = c"dynamic_mod";
            let m_ptr = unsafe { super::PyModule_New(mod_name.as_ptr()) };
            assert!(!m_ptr.is_null());
            let m = unsafe { pyo3::Bound::from_owned_ptr(py, m_ptr.cast()) };
            assert!(m.is_instance_of::<PyModule>());

            let name_ptr = unsafe { super::PyModule_GetNameObject(m_ptr) };
            assert!(!name_ptr.is_null());
            let name = unsafe { pyo3::Bound::from_owned_ptr(py, name_ptr.cast()) };
            assert_eq!(
                name.cast::<PyString>().unwrap().to_str().unwrap(),
                "dynamic_mod"
            );

            let dict_ptr = unsafe { super::PyModule_GetDict(m_ptr) };
            assert!(!dict_ptr.is_null());
            let dict = unsafe { pyo3::Bound::from_borrowed_ptr(py, dict_ptr.cast()) };
            assert!(dict.is_instance_of::<PyDict>());
        });
    }
}
