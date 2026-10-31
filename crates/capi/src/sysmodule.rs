use crate::PyObject;
use crate::pystate::with_vm;
use crate::util::{CStrExt, FfiPtrExt};
use core::ffi::{c_char, c_int};
use rustpython_vm::AsObject;
use rustpython_vm::builtins::{PyDict, PyStr};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySys_GetAttr(name: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let name = unsafe { name.assume_borrowed_and_cast::<PyStr>(vm) }?;
        vm.sys_module.dict().get_item(name, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySys_GetAttrString(name: *const c_char) -> *mut PyObject {
    with_vm(|vm| {
        let name = unsafe { name.try_as_str(vm) }?;
        vm.sys_module.dict().get_item(name, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySys_GetObject(name: *const c_char) -> *mut PyObject {
    with_vm(|vm| {
        let Ok(name) = (unsafe { name.try_as_str(vm) }) else {
            return core::ptr::null();
        };
        vm.sys_module
            .dict()
            .get_item_opt(name, vm)
            .ok()
            .flatten()
            .map_or_default(|value| value.as_object().as_raw())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySys_GetOptionalAttr(
    name: *mut PyObject,
    result: *mut *mut PyObject,
) -> c_int {
    with_vm(|vm| {
        unsafe { *result = core::ptr::null_mut() };
        let name = unsafe { name.assume_borrowed_and_cast::<PyStr>(vm) }?;
        if let Some(value) = vm.sys_module.dict().get_item_opt(name, vm)? {
            unsafe { *result = value.into_raw().as_ptr() };
            Ok(true)
        } else {
            Ok(false)
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySys_GetOptionalAttrString(
    name: *const c_char,
    result: *mut *mut PyObject,
) -> c_int {
    with_vm(|vm| {
        unsafe { *result = core::ptr::null_mut() };
        let name = unsafe { name.try_as_str(vm) }?;
        if let Some(value) = vm.sys_module.dict().get_item_opt(name, vm)? {
            unsafe { *result = value.into_raw().as_ptr() };
            Ok(true)
        } else {
            Ok(false)
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PySys_GetXOptions() -> *mut PyObject {
    with_vm(|vm| {
        let options = vm.sys_module.dict().get_item("_xoptions", vm)?;
        let options = options.try_downcast_ref::<PyDict>(vm)?;
        Ok(options.as_object().as_raw())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySys_SetObject(name: *const c_char, value: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let name = unsafe { name.try_as_str(vm) }?;
        let dict = vm.sys_module.dict();
        if let Some(value) = unsafe { value.assume_borrowed_or_opt() } {
            dict.set_item(name, value.to_owned(), vm)
        } else {
            dict.del_item(name, vm)
        }
    })
}
