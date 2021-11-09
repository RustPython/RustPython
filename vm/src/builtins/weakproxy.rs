use super::{PyStrRef, PyTypeRef};
use crate::{
    function::OptionalArg,
    types::{Constructor, SetAttr},
    PyClassImpl, PyContext, PyObjectRef, PyObjectWeak, PyResult, PyValue, VirtualMachine,
};

#[pyclass(module = false, name = "weakproxy")]
#[derive(Debug)]
pub struct PyWeakProxy {
    weak: PyObjectWeak,
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

impl Constructor for PyWeakProxy {
    type Args = WeakProxyNewArgs;

    fn py_new(
        cls: PyTypeRef,
        Self::Args { referent, callback }: Self::Args,
        vm: &VirtualMachine,
    ) -> PyResult {
        // using an internal subclass as the class prevents us from getting the generic weakref,
        // which would mess up the weakref count
        let weak_cls = WEAK_SUBCLASS.get_or_init(|| {
            vm.ctx.new_class(
                None,
                "__weakproxy",
                &vm.ctx.types.weakref_type,
                super::PyWeak::make_slots(),
            )
        });
        // TODO: PyWeakProxy should use the same payload as PyWeak
        PyWeakProxy {
            weak: referent.downgrade_with_typ(callback.into_option(), weak_cls.clone(), vm)?,
        }
        .into_pyresult_with_type(vm, cls)
    }
}

crate::common::static_cell! {
    static WEAK_SUBCLASS: PyTypeRef;
}

#[pyimpl(with(SetAttr, Constructor))]
impl PyWeakProxy {
    // TODO: callbacks
    #[pymethod(magic)]
    fn getattr(&self, attr_name: PyStrRef, vm: &VirtualMachine) -> PyResult {
        let obj = self.weak.upgrade().ok_or_else(|| {
            vm.new_exception_msg(
                vm.ctx.exceptions.reference_error.clone(),
                "weakly-referenced object no longer exists".to_owned(),
            )
        })?;
        obj.get_attr(attr_name, vm)
    }
}

impl SetAttr for PyWeakProxy {
    fn setattro(
        zelf: &crate::PyObjectView<Self>,
        attr_name: PyStrRef,
        value: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        match zelf.weak.upgrade() {
            Some(obj) => obj.call_set_attr(vm, attr_name, value),
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
