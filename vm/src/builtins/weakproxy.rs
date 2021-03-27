use super::pytype::PyTypeRef;
use super::weakref::PyWeak;
use super::PyStrRef;
use crate::function::OptionalArg;
use crate::pyobject::{PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue};
use crate::slots::SlotSetattro;
use crate::vm::VirtualMachine;

#[pyclass(module = false, name = "weakproxy")]
#[derive(Debug)]
pub struct PyWeakProxy {
    weak: PyWeak,
}

impl PyValue for PyWeakProxy {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.weakproxy_type
    }
}

#[pyimpl(with(SlotSetattro))]
impl PyWeakProxy {
    // TODO: callbacks
    #[pyslot]
    fn tp_new(
        cls: PyTypeRef,
        referent: PyObjectRef,
        callback: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<Self>> {
        if callback.is_present() {
            panic!("Passed a callback to weakproxy, but weakproxy does not yet support proxies.");
        }
        PyWeakProxy {
            weak: PyWeak::downgrade(&referent),
        }
        .into_ref_with_type(vm, cls)
    }

    #[pymethod(name = "__getattr__")]
    fn getattr(&self, attr_name: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        match self.weak.upgrade() {
            Some(obj) => vm.get_attribute(obj, attr_name),
            None => Err(vm.new_exception_msg(
                vm.ctx.exceptions.reference_error.clone(),
                "weakly-referenced object no longer exists".to_owned(),
            )),
        }
    }
}

impl SlotSetattro for PyWeakProxy {
    fn setattro(
        zelf: &PyRef<Self>,
        attr_name: PyStrRef,
        value: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        match zelf.weak.upgrade() {
            Some(obj) => vm.call_set_attr(&obj, attr_name, value),
            None => Err(vm.new_exception_msg(
                vm.ctx.exceptions.reference_error.clone(),
                "weakly-referenced object no longer exists".to_owned(),
            )),
        }
    }
}

pub fn init(context: &PyContext) {
    PyWeakProxy::extend_class(&context, &context.types.weakproxy_type);
}
