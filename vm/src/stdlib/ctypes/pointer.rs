use std::fmt;

use crate::builtins::PyTypeRef;
use crate::VirtualMachine;

use crate::stdlib::ctypes::basics::PyCData;

#[pyclass(module = "_ctypes", name = "_Pointer", base = "PyCData")]
pub struct PyCPointer {}

impl fmt::Debug for PyCPointer {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "_Pointer {{}}")
    }
}

impl PyPayload for PyCPointer {
    fn class(_vm: &VirtualMachine) -> &PyTypeRef {
        Self::static_type()
    }
}

// impl PyCDataMethods for PyCPointer {
//     fn from_param(zelf: PyRef<Self>, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {

//     }
// }

#[pyclass(flags(BASETYPE))]
impl PyCPointer {}

pub fn POINTER(cls: PyTypeRef) {}

pub fn pointer_fn(inst: PyObjectRef) {}
