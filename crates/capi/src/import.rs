use crate::util::{CStrExt, FfiPtrExt};
use crate::{PyObject, pystate::with_vm};
use core::ffi::c_char;
use rustpython_vm::builtins::{PyCode, PyStr};
use rustpython_vm::import::{add_module, import_code_obj};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyImport_Import(name: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let name = unsafe { name.assume_borrowed_and_cast::<PyStr>(vm) }?;
        let _ = vm.import(name, 0)?;

        vm.sys_module
            .get_attr(rustpython_vm::identifier!(vm, modules), vm)?
            .get_item(name, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyImport_AddModuleRef(name: *const c_char) -> *mut PyObject {
    with_vm(|vm| {
        let name = unsafe { name.try_as_str(vm) }?;

        add_module(vm, name)
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
        let code = unsafe { co.assume_borrowed_and_cast::<PyCode>(vm) }?;
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
