use crate::{PyObject, with_vm};
use crate::handles::resolve_object_handle;
use core::ffi::{CStr, c_char};
use rustpython_vm::builtins::PyStr;

#[unsafe(no_mangle)]
pub extern "C" fn PyImport_Import(name: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let name = unsafe { (&*resolve_object_handle(name)).try_downcast_ref::<PyStr>(vm)? };
        vm.import(name, 0)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyImport_AddModuleRef(name: *const c_char) -> *mut PyObject {
    with_vm(|vm| {
        let name = unsafe { CStr::from_ptr(name) }
            .to_str()
            .expect("Name is not valid UTF-8");

        // TODO check if module already exists and return it if so, instead of creating a new one

        vm.new_module(name, vm.ctx.new_dict(), None)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyImport_AddModule(name: *const c_char) -> *mut PyObject {
    PyImport_AddModuleRef(name)
}

#[cfg(test)]
mod tests {
    use pyo3::prelude::*;

    #[test]
    fn test_import() {
        Python::attach(|py| {
            let _module = py.import("sys").unwrap();
        })
    }
}
