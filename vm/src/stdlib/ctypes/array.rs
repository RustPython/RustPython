use std::fmt;

use crate::builtins::PyTypeRef;
use crate::pyobject::{PyValue, StaticType};
use crate::VirtualMachine;

use crate::stdlib::ctypes::basics::PyCData;

#[pyclass(module = "_ctypes", name = "Array", base = "PyCData")]
pub struct PyCArray {}

impl fmt::Debug for PyCArray {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "PyCArray {{}}")
    }
}

impl PyValue for PyCArray {
    fn class(_vm: &VirtualMachine) -> &PyTypeRef {
        Self::static_type()
    }
}
