use super::objtype::PyClassRef;
use crate::pyobject::{PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

#[pyclass]
#[derive(Clone, Debug)]
pub struct PyStaticMethod {
    pub callable: PyObjectRef,
}
pub type PyStaticMethodRef = PyRef<PyStaticMethod>;

impl PyValue for PyStaticMethod {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.staticmethod_type()
    }
}

#[pyimpl]
impl PyStaticMethod {
    #[pymethod(name = "__new__")]
    fn new(
        cls: PyClassRef,
        callable: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyStaticMethodRef> {
        PyStaticMethod {
            callable: callable.clone(),
        }
        .into_ref_with_type(vm, cls)
    }

    #[pymethod(name = "__get__")]
    fn get(&self, _inst: PyObjectRef, _owner: PyObjectRef, _vm: &VirtualMachine) -> PyResult {
        Ok(self.callable.clone())
    }
}

pub fn init(context: &PyContext) {
    PyStaticMethod::extend_class(context, &context.staticmethod_type);
}
