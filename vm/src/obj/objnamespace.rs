use super::objtype::PyClassRef;
use crate::function::KwArgs;
use crate::pyobject::{PyClassImpl, PyContext, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

/// A simple attribute-based namespace.
///
/// SimpleNamespace(**kwargs)
#[pyclass(module = false, name = "SimpleNamespace")]
#[derive(Debug)]
pub struct PyNamespace;

impl PyValue for PyNamespace {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.types.namespace_type.clone()
    }
}

#[pyimpl(flags(BASETYPE, HAS_DICT))]
impl PyNamespace {
    #[pyslot]
    fn tp_new(cls: PyClassRef, kwargs: KwArgs, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        let zelf = PyNamespace.into_ref_with_type(vm, cls)?;
        for (name, value) in kwargs.into_iter() {
            vm.set_attr(zelf.as_object(), name, value)?;
        }
        Ok(zelf)
    }
}

pub fn init(context: &PyContext) {
    PyNamespace::extend_class(context, &context.types.namespace_type);
}
