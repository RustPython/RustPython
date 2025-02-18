#[pyclass(name = "Union", module = "_ctypes")]
pub struct PyCUnion {}

#[pyclass(flags(BASETYPE, IMMUTABLETYPE))]
impl PyCUnion {}
