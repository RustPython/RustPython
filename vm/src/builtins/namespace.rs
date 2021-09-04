use super::pytype::PyTypeRef;
use crate::function::KwArgs;
use crate::slots::SlotConstructor;
use crate::vm::VirtualMachine;
use crate::{IntoPyObject, PyClassImpl, PyContext, PyResult, PyValue};

/// A simple attribute-based namespace.
///
/// SimpleNamespace(**kwargs)
#[pyclass(module = false, name = "SimpleNamespace")]
#[derive(Debug)]
pub struct PyNamespace;

impl PyValue for PyNamespace {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.namespace_type
    }
}

impl SlotConstructor for PyNamespace {
    type Args = KwArgs;

    fn py_new(cls: PyTypeRef, kwargs: Self::Args, vm: &VirtualMachine) -> PyResult {
        let zelf = PyNamespace.into_ref_with_type(vm, cls)?;
        for (name, value) in kwargs.into_iter() {
            vm.set_attr(zelf.as_object(), name, value)?;
        }
        Ok(zelf.into_pyobject(vm))
    }
}

#[pyimpl(flags(BASETYPE, HAS_DICT), with(SlotConstructor))]
impl PyNamespace {}

pub fn init(context: &PyContext) {
    PyNamespace::extend_class(context, &context.types.namespace_type);
}
