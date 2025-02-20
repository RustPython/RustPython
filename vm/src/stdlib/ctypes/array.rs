#[pyclass(name = "Array", module = "_ctypes")]
pub struct PyCArray {}

#[pyclass(flags(BASETYPE, IMMUTABLETYPE))]
impl PyCArray {}
