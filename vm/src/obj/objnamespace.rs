use super::objtype::PyClassRef;
use crate::function::KwArgs;
use crate::pyobject::{PyClassImpl, PyContext, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

/// A simple attribute-based namespace.
///
/// SimpleNamespace(**kwargs)
#[pyclass(name = "SimpleNamespace")]
#[derive(Debug)]
pub struct PyNamespace;

impl PyValue for PyNamespace {
    const HAVE_DICT: bool = true;
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.namespace_type()
    }
}

#[pyimpl]
impl PyNamespace {
    #[pyslot(new)]
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
