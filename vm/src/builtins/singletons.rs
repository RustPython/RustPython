use super::{PyType, PyTypeRef};
use crate::{
    atomic_func,
    class::PyClassImpl,
    convert::ToPyObject,
    protocol::PyNumberMethods,
    types::{AsNumber, Constructor},
    Context, Py, PyObjectRef, PyPayload, PyResult, VirtualMachine,
};

#[pyclass(module = false, name = "NoneType")]
#[derive(Debug)]
pub struct PyNone;

impl PyPayload for PyNone {
    fn class(vm: &VirtualMachine) -> &'static Py<PyType> {
        vm.ctx.types.none_type
    }
}

// This allows a built-in function to not return a value, mapping to
// Python's behavior of returning `None` in this situation.
impl ToPyObject for () {
    fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.none()
    }
}

impl<T: ToPyObject> ToPyObject for Option<T> {
    fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        match self {
            Some(x) => x.to_pyobject(vm),
            None => vm.ctx.none(),
        }
    }
}

impl Constructor for PyNone {
    type Args = ();

    fn py_new(_: PyTypeRef, _args: Self::Args, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.none.clone().into())
    }
}

#[pyimpl(with(Constructor, AsNumber))]
impl PyNone {
    #[pymethod(magic)]
    fn repr(&self) -> String {
        "None".to_owned()
    }

    #[pymethod(magic)]
    fn bool(&self) -> bool {
        false
    }
}

impl AsNumber for PyNone {
    fn as_number() -> &'static PyNumberMethods {
        static AS_NUMBER: PyNumberMethods = PyNumberMethods {
            boolean: atomic_func!(|_number, _vm| Ok(false)),
            ..PyNumberMethods::NOT_IMPLEMENTED
        };
        &AS_NUMBER
    }
}

#[pyclass(module = false, name = "NotImplementedType")]
#[derive(Debug)]
pub struct PyNotImplemented;

impl PyPayload for PyNotImplemented {
    fn class(vm: &VirtualMachine) -> &'static Py<PyType> {
        vm.ctx.types.not_implemented_type
    }
}

impl Constructor for PyNotImplemented {
    type Args = ();

    fn py_new(_: PyTypeRef, _args: Self::Args, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.not_implemented.clone().into())
    }
}

#[pyimpl(with(Constructor))]
impl PyNotImplemented {
    // TODO: As per https://bugs.python.org/issue35712, using NotImplemented
    // in boolean contexts will need to raise a DeprecationWarning in 3.9
    // and, eventually, a TypeError.
    #[pymethod(magic)]
    fn bool(&self) -> bool {
        true
    }

    #[pymethod(magic)]
    fn repr(&self) -> String {
        "NotImplemented".to_owned()
    }

    #[pymethod(magic)]
    fn reduce(&self) -> String {
        "NotImplemented".to_owned()
    }
}

pub fn init(context: &Context) {
    PyNone::extend_class(context, context.types.none_type);
    PyNotImplemented::extend_class(context, context.types.not_implemented_type);
}
