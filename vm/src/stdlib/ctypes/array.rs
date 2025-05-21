use crate::builtins::PyBytes;
use crate::types::Callable;
use crate::{Py, PyObjectRef, PyPayload};
use crate::{
    PyResult, VirtualMachine,
    builtins::{PyType, PyTypeRef},
    types::Constructor,
};
use crossbeam_utils::atomic::AtomicCell;
use rustpython_common::lock::PyRwLock;
use rustpython_vm::stdlib::ctypes::base::PyCData;

#[pyclass(name = "PyCArrayType", base = "PyType", module = "_ctypes")]
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

#[pyclass(flags(IMMUTABLETYPE), with(Callable, Constructor))]
impl PyCArrayType {}

#[pyclass(
    name = "Array",
    base = "PyCData",
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

    fn py_new(_cls: PyTypeRef, args: Self::Args, vm: &VirtualMachine) -> PyResult {
        Ok(Self {
            typ: PyRwLock::new(args.0),
            length: AtomicCell::new(args.1),
            value: PyRwLock::new(vm.ctx.none()),
        }
        .into_pyobject(vm))
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
        let py_bytes = value.payload::<PyBytes>().unwrap();
        let bytes = py_bytes.as_ref().to_vec();
        Ok(libffi::middle::Arg::new(&bytes))
    }
}
