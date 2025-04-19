use crate::builtins::PyType;

#[pyclass(name = "PyCPointerType", base = "PyType", module = "_ctypes")]
#[derive(PyPayload)]
pub struct PyCPointerType {
    pub(super) inner: PyCPointer,
}

#[pyclass]
impl PyCPointerType {}

#[pyclass(
    name = "_Pointer",
    base = "PyCData",
    metaclass = "PyCPointerType",
    module = "_ctypes"
)]
pub struct PyCPointer {}

#[pyclass(flags(BASETYPE, IMMUTABLETYPE))]
impl PyCPointer {}
