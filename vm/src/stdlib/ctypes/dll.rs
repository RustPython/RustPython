use ::std::sync::Arc;

use crate::builtins::pystr::PyStrRef;
use crate::builtins::pytype::PyTypeRef;
use crate::pyobject::{PyObjectRef, PyResult, PyValue};
use crate::VirtualMachine;

use crate::stdlib::ctypes::common::FUNCTIONS;

#[derive(Debug)]
pub struct SharedLibrary {
    _name: String,
    lib: &'static mut Arc<libloading::Library>,
}

impl SharedLibrary {
    pub fn get_name(&self) -> String {
        self._name
    }

    pub fn get_lib(&self) -> Arc<libloading::Library> {
        self.lib.clone()
    }
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

    Ok(vm.new_pyobj(SharedLibrary {
        _name: lib_path.to_string(),
        lib: library,
    }))
}

pub fn dlsym(
    slib: &libloading::Library,
    func_name: String,
) -> Result<*const i32, libloading::Error> {
    // This need some tweaks
    unsafe { slib.get(func_name.as_bytes())?.into_raw() as *const _ }
}
