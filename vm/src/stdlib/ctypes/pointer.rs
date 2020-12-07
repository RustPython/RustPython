use std::fmt;

use crate::builtins::PyTypeRef;
use crate::pyobject::{PyValue, StaticType};
use crate::VirtualMachine;

use crate::stdlib::ctypes::basics::PyCData;

#[pyclass(module = "_ctypes", name = "_Pointer", base = "PyCData")]
pub struct PyCPointer {}

impl fmt::Debug for PyCPointer {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "_Pointer {{}}")
    }
}

impl PyValue for PyCPointer {
    fn class(_vm: &VirtualMachine) -> &PyTypeRef {
        Self::static_type()
    }
}
