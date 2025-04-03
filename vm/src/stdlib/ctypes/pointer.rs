#[pyclass(name = "Pointer", module = "_ctypes", no_payload)]
pub struct PyCPointer {}

#[pyclass(flags(BASETYPE, IMMUTABLETYPE))]
impl PyCPointer {}
