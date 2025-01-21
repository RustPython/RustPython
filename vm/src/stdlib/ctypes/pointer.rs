use std::fmt;

use crate::PyObjectRef;
use crate::builtins::PyTypeRef;

use crate::stdlib::ctypes::basics::PyCData;

#[pyclass(module = "_ctypes", name = "_Pointer", base = "PyCData")]
pub struct PyCPointer {}

impl fmt::Debug for PyCPointer {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "_Pointer {{}}")
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
