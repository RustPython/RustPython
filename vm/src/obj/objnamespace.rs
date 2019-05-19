use crate::function::KwArgs;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{PyClassImpl, PyContext, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

/// A simple attribute-based namespace.
///
/// SimpleNamespace(**kwargs)
#[pyclass(name = "SimpleNamespace")]
#[derive(Debug)]
pub struct PyNamespace;

impl PyValue for PyNamespace {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.namespace_type()
    }
}

#[pyimpl]
impl PyNamespace {
    #[pymethod(name = "__init__")]
    fn init(zelf: PyRef<PyNamespace>, kwargs: KwArgs, vm: &VirtualMachine) -> PyResult<()> {
        for (name, value) in kwargs.into_iter() {
            vm.set_attr(zelf.as_object(), name, value)?;
        }
        Ok(())
    }
}

pub fn init(context: &PyContext) {
    PyNamespace::extend_class(context, &context.namespace_type);
}
