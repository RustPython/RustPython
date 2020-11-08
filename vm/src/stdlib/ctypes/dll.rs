extern crate libloading;

use crate::builtins::pystr::PyStrRef;
use crate::pyobject::{PyObjectRc, PyObjectRef, PyResult, StaticType};
use crate::VirtualMachine;

use crate::stdlib::ctypes::common::{SharedLibrary, CDATACACHE};

pub fn dlopen(lib_path: PyObjectRc, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
    // Match this error first
    let lib_str_path = match vm.isinstance(&lib_path, &vm.ctx.types.str_type) {
        Ok(_) => Ok(lib_path.to_string()),
        Err(e) => Err(e),
    }?;

    let result = unsafe {
        CDATACACHE
            .write()
            .get_or_insert_lib(lib_str_path.as_ref(), vm)
    };

    match result {
        Ok(lib) => Ok(lib),
        Err(_) => Err(vm.new_os_error(format!(
            "{} : cannot open shared object file: No such file or directory",
            lib_path.to_string()
        ))),
    }
}

pub fn dlsym(slib: PyObjectRc, func_name: PyStrRef, vm: &VirtualMachine) -> PyResult<*const i32> {
    // match vm.isinstance(&slib, &SharedLibrary::static_type()) {
    match slib.downcast::<SharedLibrary>() {
        Ok(lib) => {
            if let Ok(ptr) = lib.get_sym(func_name.as_ref()) {
                Ok(ptr)
            } else {
                // @TODO: Change this error message
                Err(vm.new_runtime_error(format!(
                    "Error while opening symbol {}",
                    func_name.as_ref()
                )))
            }
        }
        Err(_) => Err(vm.new_value_error("argument slib is not a valid SharedLibrary".to_string())),
    }
}
