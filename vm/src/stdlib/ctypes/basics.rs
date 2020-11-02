use crate::builtins::PyTypeRef;
use crate::pyobject::{PyValue, StaticType,PyResult};
use crate::VirtualMachine;

use crate::stdlib::ctypes::common::CDataObject;

#[pyclass(module = "_ctypes", name = "_CData", base = "CDataObject")]
#[derive(Debug)]
pub struct PyCData {

}

impl PyValue for PyCData {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        Self::static_type()
    }
}

#[pyimpl]
impl PyCData {
    #[inline]
    pub fn new() -> PyCData {
        PyCData {
            
        }
    }

    #[pymethod(name = "__init__")]
    fn init(&self, vm: &VirtualMachine) -> PyResult<()> {
        Ok(())
    }

}