#[pyclass(name = "Pointer", module = "_ctypes")]
#[derive(Debug)]
pub struct PyCPointer {}

#[pyclass(flags(BASETYPE, IMMUTABLETYPE))]
impl PyCPointer {}
