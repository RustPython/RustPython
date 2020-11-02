use crate::builtins::tuple::PyTupleRef;
use crate::builtins::pystr::PyStrRef;
use crate::builtins::pytype::PyTypeRef;
use crate::pyobject::{PyObjectRef, PyResult, PyValue, PyRef};
use crate::VirtualMachine;

use crate::stdlib::ctypes::function::PyCFuncPtr;
use crate::stdlib::ctypes::common::{FUNCTIONS};

#[derive(Debug)]
struct SharedLibrary {
    lib: &'static libloading::Library,
}

impl PyValue for SharedLibrary {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.object_type
    }
}

pub fn dlopen(lib_path: PyStrRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
    let library = unsafe { FUNCTIONS.get_or_insert_lib(lib_path.to_string()).expect("Failed to load library")};

    let shared_lib = SharedLibrary { lib : library };

    Ok(vm.new_pyobj(shared_lib))
}


// pub fn dlsym(handle: PyObjectRef, func_name: PyStrRef, argtypes: Option<PyTupleRef>, restype:Option<PyObjectRef>, vm: &VirtualMachine) -> PyResult {
//     if let Some(slib) = handle.payload::<SharedLibrary>() {
//         unsafe {
//             match slib.lib.get(func_name.as_ref().as_bytes()) {
//                 Ok(func) => return Ok(vm.new_pyobj(PyCFuncPtr::new(*func))),
//                 Err(e) => return Ok(vm.ctx.none()),
//             }
//         }
//     }
//     Ok(vm.ctx.none())
// }
