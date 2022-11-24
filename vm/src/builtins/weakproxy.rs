use once_cell::sync::Lazy;

use super::{PyStrRef, PyType, PyTypeRef, PyWeak};
use crate::{
    atomic_func,
    class::PyClassImpl,
    function::{OptionalArg, PyComparisonValue, PySetterValue},
    protocol::{PyMappingMethods, PySequenceMethods},
    types::{AsMapping, AsSequence, Comparable, Constructor, GetAttr, PyComparisonOp, SetAttr},
    Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
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

#[pyclass(with(GetAttr, SetAttr, Constructor, Comparable, AsSequence, AsMapping))]
impl PyWeakProxy {
    fn try_upgrade(&self, vm: &VirtualMachine) -> PyResult {
        self.weak.upgrade().ok_or_else(|| new_reference_error(vm))
    }

    #[pymethod(magic)]
    fn str(&self, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        self.try_upgrade(vm)?.str(vm)
    }

    fn len(&self, vm: &VirtualMachine) -> PyResult<usize> {
        self.try_upgrade(vm)?.length(vm)
    }

    #[pymethod(magic)]
    fn bool(&self, vm: &VirtualMachine) -> PyResult<bool> {
        self.try_upgrade(vm)?.is_true(vm)
    }

    #[pymethod(magic)]
    fn bytes(&self, vm: &VirtualMachine) -> PyResult {
        self.try_upgrade(vm)?.bytes(vm)
    }

    #[pymethod(magic)]
    fn repr(&self, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        self.try_upgrade(vm)?.repr(vm)
    }

    #[pymethod(magic)]
    fn contains(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        self.try_upgrade(vm)?.to_sequence(vm).contains(&needle, vm)
    }

    fn getitem(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let obj = self.try_upgrade(vm)?;
        obj.get_item(&*needle, vm)
    }

    fn setitem(
        &self,
        needle: PyObjectRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let obj = self.try_upgrade(vm)?;
        obj.set_item(&*needle, value, vm)
    }

    fn delitem(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let obj = self.try_upgrade(vm)?;
        obj.del_item(&*needle, vm)
    }
}

fn new_reference_error(vm: &VirtualMachine) -> PyRef<super::PyBaseException> {
    vm.new_exception_msg(
        vm.ctx.exceptions.reference_error.to_owned(),
        "weakly-referenced object no longer exists".to_owned(),
    )
}

impl GetAttr for PyWeakProxy {
    // TODO: callbacks
    fn getattro(zelf: &Py<Self>, name: PyStrRef, vm: &VirtualMachine) -> PyResult {
        let obj = zelf.try_upgrade(vm)?;
        obj.get_attr(name, vm)
    }
}

impl SetAttr for PyWeakProxy {
    fn setattro(
        zelf: &crate::Py<Self>,
        attr_name: PyStrRef,
        value: PySetterValue,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let obj = zelf.try_upgrade(vm)?;
        obj.call_set_attr(vm, attr_name, value)
    }
}

impl Comparable for PyWeakProxy {
    fn cmp(
        zelf: &crate::Py<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        let obj = zelf.try_upgrade(vm)?;
        Ok(PyComparisonValue::Implemented(
            obj.rich_compare_bool(other, op, vm)?,
        ))
    }
}

impl AsSequence for PyWeakProxy {
    fn as_sequence() -> &'static PySequenceMethods {
        static AS_SEQUENCE: Lazy<PySequenceMethods> = Lazy::new(|| PySequenceMethods {
            length: atomic_func!(|seq, vm| PyWeakProxy::sequence_downcast(seq).len(vm)),
            contains: atomic_func!(|seq, needle, vm| {
                PyWeakProxy::sequence_downcast(seq).contains(needle.to_owned(), vm)
            }),
            ..PySequenceMethods::NOT_IMPLEMENTED
        });
        &AS_SEQUENCE
    }
}

impl AsMapping for PyWeakProxy {
    fn as_mapping() -> &'static PyMappingMethods {
        static AS_MAPPING: PyMappingMethods = PyMappingMethods {
            length: atomic_func!(|mapping, vm| PyWeakProxy::mapping_downcast(mapping).len(vm)),
            subscript: atomic_func!(|mapping, needle, vm| {
                PyWeakProxy::mapping_downcast(mapping).getitem(needle.to_owned(), vm)
            }),
            ass_subscript: atomic_func!(|mapping, needle, value, vm| {
                let zelf = PyWeakProxy::mapping_downcast(mapping);
                if let Some(value) = value {
                    zelf.setitem(needle.to_owned(), value, vm)
                } else {
                    zelf.delitem(needle.to_owned(), vm)
                }
            }),
        };
        &AS_MAPPING
    }
}

pub fn init(context: &Context) {
    PyWeakProxy::extend_class(context, context.types.weakproxy_type);
}
