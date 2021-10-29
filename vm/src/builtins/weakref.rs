use super::{PyGenericAlias, PyTypeRef};
use crate::common::hash::PyHash;
use crate::{
    function::OptionalArg,
    types::{Callable, Comparable, Constructor, Hashable, PyComparisonOp},
    IdProtocol, PyClassImpl, PyContext, PyObject, PyObjectRef, PyObjectWeak, PyRef, PyResult,
    PyValue, TypeProtocol, VirtualMachine,
};

use crossbeam_utils::atomic::AtomicCell;

#[pyclass(module = false, name = "weakref")]
#[derive(Debug)]
pub struct PyWeak {
    referent: PyObjectWeak,
    hash: AtomicCell<Option<PyHash>>,
}

impl PyWeak {
    pub fn downgrade(obj: &PyObject) -> PyWeak {
        PyWeak {
            referent: obj.downgrade(),
            hash: AtomicCell::new(None),
        }
    }

    pub fn upgrade(&self) -> Option<PyObjectRef> {
        self.referent.upgrade()
    }
}

#[derive(FromArgs)]
pub struct WeakNewArgs {
    #[pyarg(positional)]
    referent: PyObjectRef,
    #[pyarg(positional, optional)]
    _callback: OptionalArg<PyObjectRef>,
}

impl PyValue for PyWeak {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.weakref_type
    }
}

impl Callable for PyWeak {
    type Args = ();
    #[inline]
    fn call(zelf: &crate::PyObjectView<Self>, _: Self::Args, vm: &VirtualMachine) -> PyResult {
        Ok(vm.unwrap_or_none(zelf.upgrade()))
    }
}

impl Constructor for PyWeak {
    type Args = WeakNewArgs;

    // TODO callbacks
    fn py_new(
        cls: PyTypeRef,
        Self::Args {
            referent,
            _callback,
        }: Self::Args,
        vm: &VirtualMachine,
    ) -> PyResult {
        PyWeak::downgrade(&referent).into_pyresult_with_type(vm, cls)
    }
}

#[pyimpl(with(Callable, Hashable, Comparable, Constructor), flags(BASETYPE))]
impl PyWeak {
    #[pymethod(magic)]
    fn repr(zelf: PyRef<Self>) -> String {
        let id = zelf.get_id();
        if let Some(o) = zelf.upgrade() {
            format!(
                "<weakref at {:#x}; to '{}' at {:#x}>",
                id,
                o.class().name(),
                o.get_id(),
            )
        } else {
            format!("<weakref at {:#x}; dead>", id)
        }
    }

    #[pyclassmethod(magic)]
    fn class_getitem(cls: PyTypeRef, args: PyObjectRef, vm: &VirtualMachine) -> PyGenericAlias {
        PyGenericAlias::new(cls, args, vm)
    }
}

impl Hashable for PyWeak {
    fn hash(zelf: &crate::PyObjectView<Self>, vm: &VirtualMachine) -> PyResult<PyHash> {
        match zelf.hash.load() {
            Some(hash) => Ok(hash),
            None => {
                let obj = zelf
                    .upgrade()
                    .ok_or_else(|| vm.new_type_error("weak object has gone away".to_owned()))?;
                let hash = obj.hash(vm)?;
                zelf.hash.store(Some(hash));
                Ok(hash)
            }
        }
    }
}

impl Comparable for PyWeak {
    fn cmp(
        zelf: &crate::PyObjectView<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<crate::PyComparisonValue> {
        op.eq_only(|| {
            let other = class_or_notimplemented!(Self, other);
            let both = zelf.upgrade().and_then(|s| other.upgrade().map(|o| (s, o)));
            let eq = match both {
                Some((a, b)) => vm.bool_eq(&a, &b)?,
                None => false,
            };
            Ok(eq.into())
        })
    }
}

pub fn init(context: &PyContext) {
    PyWeak::extend_class(context, &context.types.weakref_type);
}
