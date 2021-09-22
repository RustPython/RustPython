pub(crate) use _ctypes::*;

#[pymodule]
pub(crate) mod _ctypes {
    use crate::builtins::pystr::PyStrRef;
    use crate::builtins::PyIntRef;
    use crate::pyobject::{PyResult, TryFromObject};
    use crate::VirtualMachine;

    use super::super::shared_lib::libcache;

    #[pyfunction]
    pub fn dlopen(lib_path: PyStrRef, vm: &VirtualMachine) -> PyResult {
        let mut data_cache = libcache().write();

        let result = data_cache.get_or_insert_lib(lib_path.as_ref(), vm);

        match result {
            Ok(lib) => Ok(vm.new_pyobj(lib.get_pointer())),
            Err(_) => Err(vm.new_os_error(format!(
                "{} : cannot open shared object file: No such file or directory",
                lib_path.to_string()
            ))),
        }
    }

    #[pyfunction]
    pub fn dlsym(slib: PyIntRef, str_ref: PyStrRef, vm: &VirtualMachine) -> PyResult {
        let func_name = str_ref.as_ref();
        let data_cache = libcache().read();

        match data_cache.get_lib(usize::try_from_object(vm, slib.as_object().clone())?) {
            Some(l) => match l.get_sym(func_name) {
                Ok(ptr) => Ok(vm.new_pyobj(ptr as *const _ as usize)),
                Err(e) => Err(vm.new_runtime_error(e)),
            },
            _ => Err(vm.new_runtime_error("not a valid pointer to a shared library".to_string())),
        }
    }

    #[pyfunction]
    pub fn dlclose(slib: PyIntRef, vm: &VirtualMachine) -> PyResult<()> {
        let data_cache = libcache().read();

        match data_cache.get_lib(usize::try_from_object(vm, slib.as_object().clone())?) {
            Some(l) => {
                l.close();
                Ok(())
            }
            _ => Err(vm.new_runtime_error("not a valid pointer to a shared library".to_string())),
        }
    }
}
