use crate::PyObject;
use crate::object::PyTypeObject;
use crate::pystate::with_vm;
use crate::util::{CStrExt, FfiPtrExt};
use core::ffi::{c_char, c_int, c_long};
use rustpython_vm::builtins::PyModule;
use rustpython_vm::{AsObject, PyResult, VirtualMachine};

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct PyABIInfo {
    pub abiinfo_major_version: u8,
    pub abiinfo_minor_version: u8,
    pub flags: u16,
    pub build_version: u32,
    pub abi_version: u32,
}

impl PyABIInfo {
    pub(crate) fn is_supported(
        &self,
        vm: &VirtualMachine,
        module_name: Option<&str>,
    ) -> PyResult<()> {
        const PY_ABIINFO_STABLE: u16 = 0x0001;
        const PY_ABIINFO_FREETHREADED: u16 = 0x0004;

        let module_name = module_name.unwrap_or("<unknown>");

        if self.abiinfo_major_version == 0 {
            return Ok(());
        }

        if self.abiinfo_major_version > 1 {
            return Err(
                vm.new_import_error("PyABIInfo version too high", vm.ctx.new_str(module_name))
            );
        }

        if self.flags & PY_ABIINFO_STABLE == 0 {
            return Err(vm.new_import_error("not using stable ABI", vm.ctx.new_str(module_name)));
        }

        if self.flags & PY_ABIINFO_FREETHREADED == 0 {
            return Err(vm.new_import_error(
                "incompatible with free-threaded python",
                vm.ctx.new_str(module_name),
            ));
        }

        Ok(())
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyModule_AddObjectRef(
    module: *mut PyObject,
    name: *const c_char,
    value: *mut PyObject,
) -> c_int {
    with_vm(|vm| {
        let value_obj = unsafe { value.assume_borrowed_or_err(vm) }?;
        let module = unsafe { module.assume_borrowed_and_cast::<PyModule>(vm) }?;
        let name_str = unsafe { name.try_as_str(vm) }?;
        module.dict().set_item(name_str, value_obj.to_owned(), vm)?;
        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyModule_AddObject(
    module: *mut PyObject,
    name: *const c_char,
    value: *mut PyObject,
) -> c_int {
    let ret = unsafe { PyModule_AddObjectRef(module, name, value) };
    if ret == 0 && !value.is_null() {
        let _ = unsafe { value.assume_owned_or_opt() };
    }
    ret
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyModule_AddIntConstant(
    module: *mut PyObject,
    name: *const c_char,
    value: c_long,
) -> c_int {
    with_vm(|vm| {
        let module = unsafe { module.assume_borrowed_and_cast::<PyModule>(vm) }?;
        let name_str = unsafe { name.try_as_str(vm) }?;
        let py_int = vm.ctx.new_int(value);
        module.dict().set_item(name_str, py_int.into(), vm)?;
        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyModule_AddStringConstant(
    module: *mut PyObject,
    name: *const c_char,
    value: *const c_char,
) -> c_int {
    with_vm(|vm| {
        let module = unsafe { module.assume_borrowed_and_cast::<PyModule>(vm) }?;
        let name_str = unsafe { name.try_as_str(vm) }?;
        let val_str = unsafe { value.try_as_str(vm) }?;
        let py_str = vm.ctx.new_str(val_str);
        module.dict().set_item(name_str, py_str.into(), vm)?;
        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyModule_AddType(
    module: *mut PyObject,
    type_: *mut PyTypeObject,
) -> c_int {
    with_vm(|vm| {
        let module = unsafe { module.assume_borrowed_and_cast::<PyModule>(vm) }?;
        let ty = unsafe { type_.assume_borrowed() };
        let name = ty.__name__(vm);
        let slot_name = ty.slot_name();
        let name_str = name.to_str().unwrap_or(&slot_name);
        let short_name = name_str.rsplit('.').next().unwrap_or(name_str);
        module
            .dict()
            .set_item(short_name, ty.as_object().to_owned(), vm)?;
        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyABIInfo_Check(
    info: *mut PyABIInfo,
    module_name: *const c_char,
) -> c_int {
    with_vm(|vm| {
        let module_name = unsafe { module_name.try_as_str_opt(vm) }?;
        unsafe { &*info }.is_supported(vm, module_name)
    })
}

#[cfg(test)]
mod tests {
    use pyo3::prelude::*;
    use pyo3::types::{PyAnyMethods, PyInt, PyModule, PyString, PyType};

    #[test]
    fn module_add_constants_and_objects() {
        Python::attach(|py| {
            let m = PyModule::new(py, "testmod").unwrap();
            let c_str_name = c"STR_CONST";
            let c_str_val = c"hello_mod";
            let c_int_name = c"INT_CONST";
            let c_obj_name = c"OBJ_ATTR";

            unsafe {
                let ret_str = super::PyModule_AddStringConstant(
                    m.as_ptr().cast(),
                    c_str_name.as_ptr(),
                    c_str_val.as_ptr(),
                );
                assert_eq!(ret_str, 0);

                let ret_int =
                    super::PyModule_AddIntConstant(m.as_ptr().cast(), c_int_name.as_ptr(), 42);
                assert_eq!(ret_int, 0);

                let obj = PyString::new(py, "custom_val");
                let ret_obj = super::PyModule_AddObjectRef(
                    m.as_ptr().cast(),
                    c_obj_name.as_ptr(),
                    obj.as_ptr().cast(),
                );
                assert_eq!(ret_obj, 0);

                let type_ty = py.get_type::<PyInt>();
                let ret_type = super::PyModule_AddType(m.as_ptr().cast(), type_ty.as_ptr().cast());
                assert_eq!(ret_type, 0);
            }

            let str_val = m.getattr("STR_CONST").unwrap();
            assert_eq!(str_val.extract::<String>().unwrap(), "hello_mod");

            let int_val = m.getattr("INT_CONST").unwrap();
            assert_eq!(int_val.extract::<i32>().unwrap(), 42);

            let obj_val = m.getattr("OBJ_ATTR").unwrap();
            assert_eq!(obj_val.extract::<String>().unwrap(), "custom_val");

            let ty_val = m.getattr("int").unwrap();
            assert!(ty_val.is_instance_of::<PyType>());

            unsafe {
                let ret_null = super::PyModule_AddObjectRef(
                    m.as_ptr().cast(),
                    c"NULL_ATTR".as_ptr(),
                    core::ptr::null_mut(),
                );
                assert_eq!(ret_null, -1);
            }
        });
    }
}
