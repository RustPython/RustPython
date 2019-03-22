use super::objtype::PyClassRef;
use crate::pyobject::{PyContext, PyObjectRef, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

#[derive(Clone, Debug)]
pub struct PyStaticMethod {
    pub callable: PyObjectRef,
}
pub type PyStaticMethodRef = PyRef<PyStaticMethod>;

impl PyValue for PyStaticMethod {
    fn class(vm: &VirtualMachine) -> Vec<PyObjectRef> {
        vec![vm.ctx.staticmethod_type()]
    }
}

impl PyStaticMethodRef {
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

    fn get(self, _inst: PyObjectRef, _owner: PyObjectRef, _vm: &VirtualMachine) -> PyResult {
        Ok(self.callable.clone())
    }
}

pub fn init(context: &PyContext) {
    let staticmethod_type = &context.staticmethod_type;
    extend_class!(context, staticmethod_type, {
        "__get__" => context.new_rustfunc(PyStaticMethodRef::get),
        "__new__" => context.new_rustfunc(PyStaticMethodRef::new),
    });
}
