use super::objtype::{issubclass, PyClassRef};
use crate::pyobject::{PyClassImpl, PyContext, PyResult, PyValue};
use crate::vm::VirtualMachine;

pub(crate) fn init(context: &PyContext) {
    PyEllipsis::extend_class(context, &context.types.ellipsis_type);
}

#[pyclass(module = false, name = "EllipsisType")]
#[derive(Debug)]
pub struct PyEllipsis;

impl PyValue for PyEllipsis {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.types.ellipsis_type.clone()
    }
}

#[pyimpl]
impl PyEllipsis {
    #[pyslot]
    fn tp_new(cls: PyClassRef, vm: &VirtualMachine) -> PyResult {
        if issubclass(&cls, &vm.ctx.types.ellipsis_type) {
            Ok(vm.ctx.ellipsis())
        } else {
            Err(vm.new_type_error(format!(
                "ellipsis.__new__({ty}): {ty} is not a subtype of ellipsis",
                ty = cls,
            )))
        }
    }

    #[pymethod(magic)]
    fn repr(&self) -> String {
        "Ellipsis".to_owned()
    }

    #[pymethod(magic)]
    fn reduce(&self) -> String {
        "Ellipsis".to_owned()
    }
}
