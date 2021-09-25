use super::{PyStrRef, PyTypeRef, PyWeak};
use crate::{
    function::OptionalArg,
    slots::{SlotConstructor, SlotSetattro},
    PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue, VirtualMachine,
};

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

#[derive(FromArgs)]
pub struct WeakProxyNewArgs {
    #[pyarg(positional)]
    referent: PyObjectRef,
    #[pyarg(positional, optional)]
    callback: OptionalArg<PyObjectRef>,
}

impl SlotConstructor for PyWeakProxy {
    type Args = WeakProxyNewArgs;

    fn py_new(
        cls: PyTypeRef,
        Self::Args { referent, callback }: Self::Args,
        vm: &VirtualMachine,
    ) -> PyResult {
        if callback.is_present() {
            panic!("Passed a callback to weakproxy, but weakproxy does not yet support proxies.");
        }
        PyWeakProxy {
            weak: PyWeak::downgrade(&referent),
        }
        .into_pyresult_with_type(vm, cls)
    }
}

#[pyimpl(with(SlotSetattro, SlotConstructor))]
impl PyWeakProxy {
    // TODO: callbacks
    #[pymethod(magic)]
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
    PyWeakProxy::extend_class(context, &context.types.weakproxy_type);
}
