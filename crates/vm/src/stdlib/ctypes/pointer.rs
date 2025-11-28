use crossbeam_utils::atomic::AtomicCell;
use num_traits::ToPrimitive;
use rustpython_common::lock::PyRwLock;

use crate::builtins::{PyType, PyTypeRef};
use crate::convert::ToPyObject;
use crate::protocol::PyNumberMethods;
use crate::stdlib::ctypes::PyCData;
use crate::types::AsNumber;
use crate::{PyObjectRef, PyResult, VirtualMachine};

#[pyclass(name = "PyCPointerType", base = PyType, module = "_ctypes")]
#[derive(PyPayload, Debug)]
pub struct PyCPointerType {
    #[allow(dead_code)]
    pub(crate) inner: PyCPointer,
}

#[pyclass(flags(IMMUTABLETYPE), with(AsNumber))]
impl PyCPointerType {
    #[pymethod]
    fn __mul__(cls: PyTypeRef, n: isize, vm: &VirtualMachine) -> PyResult {
        use super::array::{PyCArray, PyCArrayType};
        if n < 0 {
            return Err(vm.new_value_error(format!("Array length must be >= 0, not {n}")));
        }
        Ok(PyCArrayType {
            inner: PyCArray {
                typ: PyRwLock::new(cls),
                length: AtomicCell::new(n as usize),
                value: PyRwLock::new(vm.ctx.none()),
            },
        }
        .to_pyobject(vm))
    }
}

impl AsNumber for PyCPointerType {
    fn as_number() -> &'static PyNumberMethods {
        static AS_NUMBER: PyNumberMethods = PyNumberMethods {
            multiply: Some(|a, b, vm| {
                let cls = a
                    .downcast_ref::<PyType>()
                    .ok_or_else(|| vm.new_type_error("expected type".to_owned()))?;
                let n = b
                    .try_index(vm)?
                    .as_bigint()
                    .to_isize()
                    .ok_or_else(|| vm.new_overflow_error("array size too large".to_owned()))?;
                PyCPointerType::__mul__(cls.to_owned(), n, vm)
            }),
            ..PyNumberMethods::NOT_IMPLEMENTED
        };
        &AS_NUMBER
    }
}

#[pyclass(
    name = "_Pointer",
    base = PyCData,
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
