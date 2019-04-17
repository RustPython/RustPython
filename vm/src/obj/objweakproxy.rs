use super::objweakref::PyWeak;
use crate::function::OptionalArg;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{PyContext, PyObjectRef, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

#[derive(Debug)]
pub struct PyWeakProxy {
    weak: PyWeak,
}

impl PyValue for PyWeakProxy {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.weakproxy_type()
    }
}

pub type PyWeakProxyRef = PyRef<PyWeakProxy>;

impl PyWeakProxyRef {
    // TODO callbacks
    fn create(
        cls: PyClassRef,
        referent: PyObjectRef,
        _callback: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        PyWeakProxy {
            weak: PyWeak::downgrade(&referent),
        }
        .into_ref_with_type(vm, cls)
    }

    fn getattr(self, attr_name: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        match self.weak.upgrade() {
            Some(obj) => vm.get_attribute(obj, attr_name),
            None => Err(vm.new_exception(
                vm.ctx.exceptions.reference_error.clone(),
                "weakly-referenced object no longer exists".to_string(),
            )),
        }
    }
}

pub fn init(context: &PyContext) {
    extend_class!(context, &context.weakproxy_type, {
        "__new__" => context.new_rustfunc(PyWeakProxyRef::create),
        "__getattr__" => context.new_rustfunc(PyWeakProxyRef::getattr),
    });
}
