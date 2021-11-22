use super::{PyGenericAlias, PyTypeRef};
use crate::common::atomic::{Ordering, Radium};
use crate::common::hash::{self, PyHash};
use crate::{
    function::OptionalArg,
    types::{Callable, Comparable, Constructor, Hashable, PyComparisonOp},
    IdProtocol, PyClassImpl, PyContext, PyObject, PyObjectRef, PyRef, PyResult, PyValue,
    TypeProtocol, VirtualMachine,
};

pub use crate::pyobjectrc::PyWeak;

#[derive(FromArgs)]
pub struct WeakNewArgs {
    #[pyarg(positional)]
    referent: PyObjectRef,
    #[pyarg(positional, optional)]
    callback: OptionalArg<PyObjectRef>,
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

    fn py_new(
        cls: PyTypeRef,
        Self::Args { referent, callback }: Self::Args,
        vm: &VirtualMachine,
    ) -> PyResult {
        let weak = referent.downgrade_with_typ(callback.into_option(), cls, vm)?;
        Ok(weak.into_object())
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
        let hash = match zelf.hash.load(Ordering::Relaxed) {
            hash::SENTINEL => {
                let obj = zelf
                    .upgrade()
                    .ok_or_else(|| vm.new_type_error("weak object has gone away".to_owned()))?;
                let hash = obj.hash(vm)?;
                match Radium::compare_exchange(
                    &zelf.hash,
                    hash::SENTINEL,
                    hash::fix_sentinel(hash),
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => hash,
                    Err(prev_stored) => prev_stored,
                }
            }
            hash => hash,
        };
        Ok(hash)
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
                None => zelf.is(other),
            };
            Ok(eq.into())
        })
    }
}

pub fn init(context: &PyContext) {
    PyWeak::extend_class(context, &context.types.weakref_type);
}
