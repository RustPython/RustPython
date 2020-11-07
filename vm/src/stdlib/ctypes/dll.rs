extern crate libloading;

use crate::builtins::pystr::PyStrRef;
use crate::pyobject::{PyObjectRef, PyResult};
use crate::VirtualMachine;

use crate::stdlib::ctypes::common::{SharedLibrary, CDATACACHE};

pub fn dlopen(lib_path: PyStrRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
    let library = unsafe {
        CDATACACHE
            .write()
            .get_or_insert_lib(lib_path.as_ref(), vm)
            .expect("Failed to load library")
    };
    Ok(library)
}

pub fn dlsym(slib: PyObjectRef, func_name: PyStrRef, vm: &VirtualMachine) -> PyResult<*const i32> {
    // cast to PyRef<SharedLibrary>
    match vm.cast(slib, SharedLibrary) {
        Ok(lib) => {
            let ptr_res = unsafe { lib.get_lib().get(func_name.as_ref().as_bytes()).map(|f| *f) };
            if ptr_res.is_err() {
                Err(vm.new_runtime_error(format!(
                    "Error while opening symbol {}",
                    func_name.as_ref()
                )))
            } else {
                Ok(ptr_res.unwrap())
            }
        }
        Err(_) => Err(vm.new_value_error("argument slib is not a valid SharedLibrary".to_string())),
    }
}
