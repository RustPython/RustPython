use super::PyTypeRef;
use crate::{
    function::IntoPyObject, slots::SlotConstructor, PyClassImpl, PyContext, PyObjectRef, PyResult,
    PyValue, TypeProtocol, VirtualMachine,
};

#[pyclass(module = false, name = "NoneType")]
#[derive(Debug)]
pub struct PyNone;

impl PyValue for PyNone {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.none_type
    }
}

// This allows a built-in function to not return a value, mapping to
// Python's behavior of returning `None` in this situation.
impl IntoPyObject for () {
    fn into_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.none()
    }
}

impl<T: IntoPyObject> IntoPyObject for Option<T> {
    fn into_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        match self {
            Some(x) => x.into_pyobject(vm),
            None => vm.ctx.none(),
        }
    }
}

impl SlotConstructor for PyNone {
    type Args = ();

    fn py_new(_: PyTypeRef, _args: Self::Args, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.none.clone().into_object())
    }
}

#[pyimpl(with(SlotConstructor))]
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

#[pyclass(module = false, name = "NotImplementedType")]
#[derive(Debug)]
pub struct PyNotImplemented;

impl PyValue for PyNotImplemented {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.not_implemented_type
    }
}

impl SlotConstructor for PyNotImplemented {
    type Args = ();

    fn py_new(_: PyTypeRef, _args: Self::Args, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.not_implemented.clone().into_object())
    }
}

#[pyimpl(with(SlotConstructor))]
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
}

pub fn init(context: &PyContext) {
    PyNone::extend_class(context, &context.none.clone_class());
    PyNotImplemented::extend_class(context, &context.not_implemented.clone_class());
}
