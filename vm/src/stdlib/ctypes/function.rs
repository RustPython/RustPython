use crate::builtins::PyTypeRef;
use crate::pyobject::{PyValue, StaticType};
use crate::VirtualMachine;

#[pyclass(module = "_ctypes", name = "CFuncPtr")]
#[derive(Debug)]
pub struct PyCFuncPtr {
    ext_func: extern "C" fn(),
}

impl PyValue for PyCFuncPtr {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        Self::static_type()
    }
}

#[pyimpl]
impl PyCFuncPtr {
    #[inline]
    pub fn new(ext_func: extern "C" fn()) -> PyCFuncPtr {
        PyCFuncPtr { ext_func }
    }

    #[pymethod]
    pub fn call(&self) {
        (self.ext_func)();
    }
}
