use std::fmt;

use crate::stdlib::ctypes::basics::PyCData;

#[pyclass(module = "_ctypes", name = "Structure", base = "PyCData")]
pub struct PyCStructure {}

impl fmt::Debug for PyCStructure {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "PyCStructure {{}}")
    }
}

#[pyclass(flags(BASETYPE))]
impl PyCStructure {}
