use super::base::PyCData;

// TODO: metaclass = "UnionType"
#[pyclass(module = "_ctypes", name = "Union", base = PyCData)]
#[derive(Debug)]
pub struct PyCUnion {}

#[pyclass(flags(BASETYPE, IMMUTABLETYPE))]
impl PyCUnion {}
