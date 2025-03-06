use crate::types::Callable;
use crate::{Py, PyObjectRef, PyPayload};
use crate::{PyResult, VirtualMachine, builtins::PyTypeRef, types::Constructor};
use crossbeam_utils::atomic::AtomicCell;
use rustpython_common::lock::PyRwLock;

// TODO: make it metaclass
#[pyclass(name = "Array", module = "_ctypes")]
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
        .into_pyobject(&vm))
    }
}

impl Constructor for PyCArrayType {
    type Args = PyObjectRef;

    fn py_new(_cls: PyTypeRef, args: Self::Args, vm: &VirtualMachine) -> PyResult {
        unreachable!()
    }
}

#[pyclass(flags(IMMUTABLETYPE))]
impl PyCArrayType {}

#[pyclass(name = "Array", module = "_ctypes")]
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
        .into_pyobject(&vm))
    }
}

#[pyclass(flags(BASETYPE, IMMUTABLETYPE))]
impl PyCArray {
    #[pygetset(name = "_type_")]
    fn typ(&self) -> PyTypeRef {
        self.typ.read().clone()
    }

    #[pygetset(name = "_length_")]
    fn length(&self) -> usize {
        self.length.load()
    }

    #[pygetset(name = "_value_")]
    fn value(&self) -> PyObjectRef {
        self.value.read().clone()
    }

    #[pygetset(setter, name = "_value_")]
    fn set_value(&self, value: PyObjectRef) {
        *self.value.write() = value;
    }
}
