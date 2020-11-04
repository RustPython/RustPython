use crate::builtins::PyTypeRef;
use crate::pyobject::{PyResult, PyValue, StaticType};
use crate::VirtualMachine;

use crate::stdlib::ctypes::common::CDataObject;

#[pyclass(module = "_ctypes", name = "_CData", base = "CDataObject")]
#[derive(Debug)]
pub struct PyCData {
    _type_: String,
}

impl PyValue for PyCData {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        Self::static_type()
    }
}

#[pyimpl]
impl PyCData {
    #[pymethod(name = "__init__")]
    fn init(&self, vm: &VirtualMachine) -> PyResult<()> {
        Ok(())
    }
}
