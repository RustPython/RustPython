use crate::builtins::pystr::PyStrRef;
use crate::builtins::pytype::PyTypeRef;
use crate::builtins::tuple::PyTupleRef;
use crate::pyobject::{PyObjectRef, PyRef, PyResult, PyValue};
use crate::VirtualMachine;

use crate::stdlib::ctypes::common::FUNCTIONS;
use crate::stdlib::ctypes::function::PyCFuncPtr;

#[derive(Debug)]
pub struct SharedLibrary {
    lib: &'static libloading::Library,
}

impl PyValue for SharedLibrary {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.object_type
    }
}

pub fn dlopen(lib_path: PyStrRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
    let library = unsafe {
        FUNCTIONS
            .get_or_insert_lib(lib_path.to_string())
            .expect("Failed to load library")
    };

    let shared_lib = SharedLibrary { lib: library };

    Ok(vm.new_pyobj(shared_lib))
}

pub fn dlsym(
    slib: &SharedLibrary,
    func_name: PyStrRef,
) -> Result<extern "C" fn(), libloading::Error> {
    unsafe {
        slib.lib
            .get(func_name.as_ref().as_bytes())
            .map(|func| *func)
    }
}
