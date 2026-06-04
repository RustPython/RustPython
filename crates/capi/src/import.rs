use crate::{PyObject, pystate::with_vm};
use core::ffi::{CStr, c_char};
use rustpython_vm::builtins::{PyCode, PyDict, PyModule, PyStr};
use rustpython_vm::import::import_code_obj;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyImport_Import(name: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let name = unsafe { (&*name).try_downcast_ref::<PyStr>(vm)? };
        vm.import(name, 0)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyImport_AddModuleRef(name: *const c_char) -> *mut PyObject {
    with_vm(|vm| {
        let name = unsafe { CStr::from_ptr(name) }
            .to_str()
            .map_err(|_| vm.new_system_error("PyImport_AddModuleRef called with non utf8 name"))?;

        let sys_modules = vm
            .sys_module
            .get_attr(rustpython_vm::identifier!(vm, modules), vm)?;

        sys_modules
            .try_downcast_ref::<PyDict>(vm)?
            .get_item_opt(name, vm)?
            .map_or_else(
                || {
                    let module = vm.new_module(name, vm.ctx.new_dict(), None);
                    sys_modules.set_item(name, module.clone().into(), vm)?;
                    Ok(module)
                },
                |module| {
                    let module = module.try_downcast_ref::<PyModule>(vm)?;
                    Ok(module.to_owned())
                },
            )
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyImport_ExecCodeModuleEx(
    name: *const c_char,
    co: *mut PyObject,
    pathname: *const c_char,
) -> *mut PyObject {
    with_vm(|vm| {
        let name = unsafe { CStr::from_ptr(name) }.to_str().map_err(|_| {
            vm.new_system_error("PyImport_ExecCodeModuleEx called with non utf8 name")
        })?;
        let code = unsafe { &*co }.try_downcast_ref::<PyCode>(vm)?;
        let module = import_code_obj(vm, name, code.to_owned(), false)?;

        if !pathname.is_null() {
            let pathname = unsafe { CStr::from_ptr(pathname) }.to_str().map_err(|_| {
                vm.new_system_error("PyImport_ExecCodeModuleEx called with non utf8 pathname")
            })?;
            module.set_attr("__file__", vm.ctx.new_str(pathname), vm)?;
        }

        Ok(module)
    })
}

#[cfg(test)]
mod tests {
    use pyo3::prelude::*;

    #[test]
    fn import() {
        Python::attach(|py| {
            let _module = py.import("sys").unwrap();
        })
    }

    #[test]
    fn import_stdlib() {
        Python::attach(|py| {
            let _module = py.import("types").unwrap();
        })
    }
}
