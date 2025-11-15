#[pyclass(name = "Pointer", module = "_ctypes")]
pub struct PyCPointer {}

#[pyclass(flags(BASETYPE, IMMUTABLETYPE))]
impl PyCPointer {}
