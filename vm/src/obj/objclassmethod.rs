use super::objtype::PyClassRef;
use crate::pyobject::{PyContext, PyObjectRef, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

#[derive(Clone, Debug)]
pub struct PyClassMethod {
    pub callable: PyObjectRef,
}
pub type PyClassMethodRef = PyRef<PyClassMethod>;

impl PyValue for PyClassMethod {
    const HAVE_DICT: bool = true;

    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.classmethod_type()
    }
}

impl PyClassMethodRef {
    fn new(
        cls: PyClassRef,
        callable: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyClassMethodRef> {
        PyClassMethod {
            callable: callable.clone(),
        }
        .into_ref_with_type(vm, cls)
    }

    fn get(self, _inst: PyObjectRef, owner: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        Ok(vm
            .ctx
            .new_bound_method(self.callable.clone(), owner.clone()))
    }
}

pub fn init(context: &PyContext) {
    let classmethod_type = &context.classmethod_type;
    extend_class!(context, classmethod_type, {
        "__get__" => context.new_rustfunc(PyClassMethodRef::get),
        "__new__" => context.new_rustfunc(PyClassMethodRef::new)
    });
}
