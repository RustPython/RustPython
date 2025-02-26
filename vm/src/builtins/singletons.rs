use super::{PyStrRef, PyType, PyTypeRef};
use crate::{
    Context, Py, PyObjectRef, PyPayload, PyResult, VirtualMachine,
    class::PyClassImpl,
    convert::ToPyObject,
    protocol::PyNumberMethods,
    types::{AsNumber, Constructor, Representable},
};

#[pyclass(module = false, name = "NoneType")]
#[derive(Debug)]
pub struct PyNone;

impl PyPayload for PyNone {
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.none_type
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

#[pyclass(with(Constructor, AsNumber, Representable))]
impl PyNone {
    #[pymethod(magic)]
    fn bool(&self) -> bool {
        false
    }
}

impl Representable for PyNone {
    #[inline]
    fn repr(_zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        Ok(vm.ctx.names.None.to_owned())
    }

    #[cold]
    fn repr_str(_zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
        unreachable!("use repr instead")
    }
}

impl AsNumber for PyNone {
    fn as_number() -> &'static PyNumberMethods {
        static AS_NUMBER: PyNumberMethods = PyNumberMethods {
            boolean: Some(|_number, _vm| Ok(false)),
            ..PyNumberMethods::NOT_IMPLEMENTED
        };
        &AS_NUMBER
    }
}

#[pyclass(module = false, name = "NotImplementedType")]
#[derive(Debug)]
pub struct PyNotImplemented;

impl PyPayload for PyNotImplemented {
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.not_implemented_type
    }
}

impl Constructor for PyNotImplemented {
    type Args = ();

    fn py_new(_: PyTypeRef, _args: Self::Args, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.not_implemented.clone().into())
    }
}

#[pyclass(with(Constructor))]
impl PyNotImplemented {
    // TODO: As per https://bugs.python.org/issue35712, using NotImplemented
    // in boolean contexts will need to raise a DeprecationWarning in 3.9
    // and, eventually, a TypeError.
    #[pymethod(magic)]
    fn bool(&self) -> bool {
        true
    }

    #[pymethod(magic)]
    fn reduce(&self, vm: &VirtualMachine) -> PyStrRef {
        vm.ctx.names.NotImplemented.to_owned()
    }
}

impl Representable for PyNotImplemented {
    #[inline]
    fn repr(_zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        Ok(vm.ctx.names.NotImplemented.to_owned())
    }

    #[cold]
    fn repr_str(_zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
        unreachable!("use repr instead")
    }
}

pub fn init(context: &Context) {
    PyNone::extend_class(context, context.types.none_type);
    PyNotImplemented::extend_class(context, context.types.not_implemented_type);
}
