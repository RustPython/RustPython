use super::base::PyCData;

// TODO: metaclass = "UnionType"
#[pyclass(module = "_ctypes", name = "Union", base = "PyCData", no_payload)]
pub struct PyCUnion {}

#[pyclass(flags(BASETYPE, IMMUTABLETYPE))]
impl PyCUnion {}
