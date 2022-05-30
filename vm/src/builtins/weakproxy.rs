use super::{PyStrRef, PyType, PyTypeRef, PyWeak};
use crate::{
    class::PyClassImpl,
    function::OptionalArg,
    types::{Constructor, SetAttr},
    Context, Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
};

#[pyclass(module = false, name = "weakproxy")]
#[derive(Debug)]
pub struct PyWeakProxy {
    weak: PyRef<PyWeak>,
}

impl PyPayload for PyWeakProxy {
    fn class(vm: &VirtualMachine) -> &'static Py<PyType> {
        vm.ctx.types.weakproxy_type
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
                vm.ctx.types.weakref_type.to_owned(),
                super::PyWeak::make_slots(),
            )
        });
        // TODO: PyWeakProxy should use the same payload as PyWeak
        PyWeakProxy {
            weak: referent.downgrade_with_typ(callback.into_option(), weak_cls.clone(), vm)?,
        }
        .into_ref_with_type(vm, cls)
        .map(Into::into)
    }
}

crate::common::static_cell! {
    static WEAK_SUBCLASS: PyTypeRef;
}

#[pyclass(with(SetAttr, Constructor))]
impl PyWeakProxy {
    // TODO: callbacks
    #[pymethod(magic)]
    fn getattr(&self, attr_name: PyStrRef, vm: &VirtualMachine) -> PyResult {
        let obj = self.weak.upgrade().ok_or_else(|| new_reference_error(vm))?;
        obj.get_attr(attr_name, vm)
    }
    #[pymethod(magic)]
    fn str(&self, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        match self.weak.upgrade() {
            Some(obj) => obj.str(vm),
            None => Err(new_reference_error(vm)),
        }
    }
}

fn new_reference_error(vm: &VirtualMachine) -> PyRef<super::PyBaseException> {
    vm.new_exception_msg(
        vm.ctx.exceptions.reference_error.to_owned(),
        "weakly-referenced object no longer exists".to_owned(),
    )
}

impl SetAttr for PyWeakProxy {
    fn setattro(
        zelf: &crate::Py<Self>,
        attr_name: PyStrRef,
        value: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        match zelf.weak.upgrade() {
            Some(obj) => obj.call_set_attr(vm, attr_name, value),
            None => Err(vm.new_exception_msg(
                vm.ctx.exceptions.reference_error.to_owned(),
                "weakly-referenced object no longer exists".to_owned(),
            )),
        }
    }
}

pub fn init(context: &Context) {
    PyWeakProxy::extend_class(context, context.types.weakproxy_type);
}
