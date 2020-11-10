extern crate libloading;

use crate::builtins::pystr::{PyStr};
use crate::pyobject::{PyObjectRc, PyObjectRef, PyResult};
use crate::VirtualMachine;

use crate::stdlib::ctypes::common::{SharedLibrary, CDATACACHE};

pub fn dlopen(lib_path: PyObjectRc, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
    // Match this error first
    let lib_str_path = match vm.isinstance(&lib_path, &vm.ctx.types.str_type) {
        Ok(_) => Ok(lib_path.to_string()),
        Err(e) => Err(e),
    }?;

    let mut data_cache = CDATACACHE.write();

    let result = data_cache.get_or_insert_lib(lib_str_path.as_ref(), vm);

    match result {
        Ok(lib) => Ok(lib.clone().into_object()),
        Err(_) => Err(vm.new_os_error(format!(
            "{} : cannot open shared object file: No such file or directory",
            lib_path.to_string()
        ))),
    }
}

pub fn dlsym(slib: PyObjectRc, func: PyObjectRc, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
    // match vm.isinstance(&slib, &SharedLibrary::static_type()) {
    if !vm.isinstance(&func, &vm.ctx.types.str_type)? {
        return Err(vm.new_value_error("argument func_name must be str".to_string()));
    }   
    
    let func_name = func.downcast::<PyStr>().unwrap().as_ref();

    match slib.downcast::<SharedLibrary>() {
        Ok(lib) => {
            if let Ok(ptr) = lib.get_sym(func_name) {
                Ok(vm.new_pyobj(ptr as isize))

            } else {
                // @TODO: Change this error message
                Err(vm.new_runtime_error(format!(
                    "Error while opening symbol {}",
                    func_name
                )))
            }
        }
        Err(_) => Err(vm.new_value_error("argument slib is not a valid SharedLibrary".to_string())),
    }
}
