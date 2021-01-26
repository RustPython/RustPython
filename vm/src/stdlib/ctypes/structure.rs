use std::fmt;

use crate::builtins::PyTypeRef;
use crate::pyobject::{PyValue, StaticType};
use crate::VirtualMachine;

use crate::stdlib::ctypes::basics::PyCData;

#[pyclass(module = "_ctypes", name = "Structure", base = "PyCData")]
pub struct PyCStructure {}

impl fmt::Debug for PyCStructure {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "PyCStructure {{}}")
    }
}

impl PyValue for PyCStructure {
    fn class(_vm: &VirtualMachine) -> &PyTypeRef {
        Self::static_type()
    }
}

#[pyimpl(flags(BASETYPE))]
impl PyCStructure {}
