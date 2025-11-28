use crate::builtins::PyBytes;
use crate::convert::ToPyObject;
use crate::protocol::PyNumberMethods;
use crate::types::{AsNumber, Callable};
use crate::{Py, PyObjectRef, PyPayload};
use crate::{
    PyResult, VirtualMachine,
    builtins::{PyType, PyTypeRef},
    types::Constructor,
};
use crossbeam_utils::atomic::AtomicCell;
use num_traits::ToPrimitive;
use rustpython_common::lock::PyRwLock;
use rustpython_vm::stdlib::ctypes::base::PyCData;

#[pyclass(name = "PyCArrayType", base = PyType, module = "_ctypes")]
#[derive(PyPayload)]
pub struct PyCArrayType {
    pub(super) inner: PyCArray,
}

impl std::fmt::Debug for PyCArrayType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PyCArrayType")
            .field("inner", &self.inner)
            .finish()
    }
}

impl Callable for PyCArrayType {
    type Args = ();
    fn call(zelf: &Py<Self>, _args: Self::Args, vm: &VirtualMachine) -> PyResult {
        Ok(PyCArray {
            typ: PyRwLock::new(zelf.inner.typ.read().clone()),
            length: AtomicCell::new(zelf.inner.length.load()),
            value: PyRwLock::new(zelf.inner.value.read().clone()),
        }
        .into_pyobject(vm))
    }
}

impl Constructor for PyCArrayType {
    type Args = PyObjectRef;

    fn py_new(_cls: PyTypeRef, _args: Self::Args, _vm: &VirtualMachine) -> PyResult {
        unreachable!()
    }
}

#[pyclass(flags(IMMUTABLETYPE), with(Callable, Constructor, AsNumber))]
impl PyCArrayType {
    #[pymethod]
    fn __mul__(zelf: &Py<Self>, n: isize, vm: &VirtualMachine) -> PyResult {
        if n < 0 {
            return Err(vm.new_value_error(format!("Array length must be >= 0, not {n}")));
        }
        // Create a nested array type: (inner_type * inner_length) * n
        // The new array's element type is the current array type
        let inner_type = zelf.inner.typ.read().clone();
        let inner_length = zelf.inner.length.load();

        // Create a new array type where the element is the current array
        Ok(PyCArrayType {
            inner: PyCArray {
                typ: PyRwLock::new(inner_type),
                length: AtomicCell::new(inner_length * n as usize),
                value: PyRwLock::new(vm.ctx.none()),
            },
        }
        .to_pyobject(vm))
    }
}

impl AsNumber for PyCArrayType {
    fn as_number() -> &'static PyNumberMethods {
        static AS_NUMBER: PyNumberMethods = PyNumberMethods {
            multiply: Some(|a, b, vm| {
                let zelf = a
                    .downcast_ref::<PyCArrayType>()
                    .ok_or_else(|| vm.new_type_error("expected PyCArrayType".to_owned()))?;
                let n = b
                    .try_index(vm)?
                    .as_bigint()
                    .to_isize()
                    .ok_or_else(|| vm.new_overflow_error("array size too large".to_owned()))?;
                PyCArrayType::__mul__(zelf, n, vm)
            }),
            ..PyNumberMethods::NOT_IMPLEMENTED
        };
        &AS_NUMBER
    }
}

#[pyclass(
    name = "Array",
    base = PyCData,
    metaclass = "PyCArrayType",
    module = "_ctypes"
)]
#[derive(PyPayload)]
pub struct PyCArray {
    pub(super) typ: PyRwLock<PyTypeRef>,
    pub(super) length: AtomicCell<usize>,
    pub(super) value: PyRwLock<PyObjectRef>,
}

impl std::fmt::Debug for PyCArray {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PyCArray")
            .field("typ", &self.typ)
            .field("length", &self.length)
            .finish()
    }
}

impl Constructor for PyCArray {
    type Args = (PyTypeRef, usize);

    fn py_new(cls: PyTypeRef, args: Self::Args, vm: &VirtualMachine) -> PyResult {
        Self {
            typ: PyRwLock::new(args.0),
            length: AtomicCell::new(args.1),
            value: PyRwLock::new(vm.ctx.none()),
        }
        .into_ref_with_type(vm, cls)
        .map(Into::into)
    }
}

#[pyclass(flags(BASETYPE, IMMUTABLETYPE), with(Constructor))]
impl PyCArray {
    #[pygetset(name = "_type_")]
    fn typ(&self) -> PyTypeRef {
        self.typ.read().clone()
    }

    #[pygetset(name = "_length_")]
    fn length(&self) -> usize {
        self.length.load()
    }

    #[pygetset]
    fn value(&self) -> PyObjectRef {
        self.value.read().clone()
    }

    #[pygetset(setter)]
    fn set_value(&self, value: PyObjectRef) {
        *self.value.write() = value;
    }
}

impl PyCArray {
    #[allow(unused)]
    pub fn to_arg(&self, _vm: &VirtualMachine) -> PyResult<libffi::middle::Arg> {
        let value = self.value.read();
        let py_bytes = value.downcast_ref::<PyBytes>().unwrap();
        let bytes = py_bytes.payload().to_vec();
        Ok(libffi::middle::Arg::new(&bytes))
    }
}
