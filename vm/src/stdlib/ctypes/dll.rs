extern crate libloading;

use crate::common::rc::PyRc;
use crate::builtins::pystr::PyStrRef;
use crate::pyobject::{PyRef, PyObjectRef, PyResult};
use crate::VirtualMachine;

use crate::stdlib::ctypes::common::{SharedLibrary,FUNCTIONS};


pub fn dlopen(lib_path: PyStrRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
    let library = unsafe {
        FUNCTIONS
            .write()
            .get_or_insert_lib(lib_path.as_ref())
            .expect("Failed to load library")
    };

    let box_arc = Box::new(PyRc::as_ptr(&library));
    let f_lib = unsafe { box_arc.read() };
    Ok(vm.new_pyobj(f_lib))
}

pub fn dlsym(
    slib: PyRef<SharedLibrary>,
    func_name: PyStrRef,
    vm: &VirtualMachine
) -> PyResult<*const i32> {

    let ptr_res = unsafe { 
        slib.get_lib()
        .get(func_name.as_ref().as_bytes())
        .map(|f| *f)
    };
    
    if ptr_res.is_err() {
        Err(vm.new_runtime_error(format!("Error while opening symbol {}",func_name.as_ref())))
    } else {
        Ok(ptr_res.unwrap())
    }
}
