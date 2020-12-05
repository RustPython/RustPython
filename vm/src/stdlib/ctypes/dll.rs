extern crate libloading;

use crate::builtins::pystr::PyStr;
use crate::builtins::PyInt;
use crate::pyobject::{PyObjectRc, PyObjectRef, PyResult, TypeProtocol};
use crate::VirtualMachine;

use crate::stdlib::ctypes::shared_lib::{SharedLibrary, LIBCACHE};

pub fn dlopen(lib_path: PyObjectRc, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
    // Match this error first
    let lib_str_path = match vm.isinstance(&lib_path, &vm.ctx.types.str_type) {
        Ok(_) => Ok(lib_path.to_string()),
        Err(e) => Err(e),
    }?;

    let mut data_cache = LIBCACHE.write();

    let result = data_cache.get_or_insert_lib(lib_str_path.as_ref(), vm);

    match result {
        Ok(lib) => Ok(lib.clone().into_object()),
        Err(_) => Err(vm.new_os_error(format!(
            "{} : cannot open shared object file: No such file or directory",
            lib_path.to_string()
        ))),
    }
}

pub fn dlsym(slib: PyObjectRc, func: PyObjectRc, vm: &VirtualMachine) -> PyResult<PyInt> {
    if !vm.isinstance(&func, &vm.ctx.types.str_type)? {
        return Err(vm.new_value_error("second argument (func) must be str".to_string()));
    }
    let str_ref = func.downcast::<PyStr>().unwrap();
    let func_name = str_ref.as_ref();

    match slib.clone().downcast::<SharedLibrary>() {
        Ok(lib) => {
            match lib.get_sym(func_name) {
                Ok(ptr) => Ok(PyInt::from(ptr as *const _ as usize)),
                Err(e) => Err(vm
                    .new_runtime_error(format!("Error while opening symbol {}: {}", func_name, e))),
            }
        }
        Err(_) => Err(vm.new_type_error(format!(
            "a SharedLibrary is required, found {}",
            slib.class().name
        ))),
    }
}

pub fn dlclose(slib: PyObjectRc, vm: &VirtualMachine) -> PyResult {
    match slib.clone().downcast::<SharedLibrary>() {
        Ok(lib) => {
            lib.close();
            Ok(vm.ctx.none())
        }
        Err(_) => Err(vm.new_type_error(format!(
            "a SharedLibrary is required, found {}",
            slib.class().name
        ))),
    }
}
