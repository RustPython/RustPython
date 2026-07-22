use crate::util::CStrExt;
use crate::{PyObject, pystate::with_vm};
use core::ffi::c_char;
use rustpython_vm::builtins::{PyCode, PyDict, PyModule, PyStr};
use rustpython_vm::import::import_code_obj;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyImport_Import(name: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let name = unsafe { (&*name).try_downcast_ref::<PyStr>(vm)? };
        vm.import(name, 0)?;

        let sys_modules = vm
            .sys_module
            .get_attr(rustpython_vm::identifier!(vm, modules), vm)?;
        let sys_modules = sys_modules.try_downcast_ref::<PyDict>(vm)?;

        if let Some(module) = sys_modules.get_item_opt(name, vm)? {
            Ok(module)
        } else {
            vm.import(name, 0)
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyImport_AddModuleRef(name: *const c_char) -> *mut PyObject {
    with_vm(|vm| {
        let name = unsafe { name.try_as_str(vm) }?;

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
        let name = unsafe { name.try_as_str(vm) }?;
        let code = unsafe { &*co }.try_downcast_ref::<PyCode>(vm)?;
        let module = import_code_obj(vm, name, code.to_owned(), false)?;

        if let Some(pathname) = unsafe { pathname.try_as_str_opt(vm) }? {
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

    #[test]
    fn import_sub_module() {
        Python::attach(|py| {
            let module = py.import("collections.abc").unwrap();
            module.getattr("Sequence").unwrap();
        })
    }
}
