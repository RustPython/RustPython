use rustpython_common::lock::PyRwLock;

use crate::builtins::PyType;
use crate::stdlib::ctypes::PyCData;
use crate::{PyObjectRef, PyResult};

#[pyclass(name = "PyCPointerType", base = "PyType", module = "_ctypes")]
#[derive(PyPayload, Debug)]
pub struct PyCPointerType {
    pub inner: PyCPointer,
}

#[pyclass]
impl PyCPointerType {}

#[pyclass(
    name = "_Pointer",
    base = "PyCData",
    metaclass = "PyCPointerType",
    module = "_ctypes"
)]
#[derive(Debug, PyPayload)]
pub struct PyCPointer {
    contents: PyRwLock<PyObjectRef>,
}

#[pyclass(flags(BASETYPE, IMMUTABLETYPE))]
impl PyCPointer {
    // TODO: not correct
    #[pygetset]
    fn contents(&self) -> PyResult<PyObjectRef> {
        let contents = self.contents.read().clone();
        Ok(contents)
    }
    #[pygetset(setter)]
    fn set_contents(&self, contents: PyObjectRef) -> PyResult<()> {
        *self.contents.write() = contents;
        Ok(())
    }
}
