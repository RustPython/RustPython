use super::{PyGenericAlias, PyType, PyTypeRef};
use crate::common::{
    atomic::{Ordering, Radium},
    hash::{self, PyHash},
};
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyResult, VirtualMachine,
    class::PyClassImpl,
    function::OptionalArg,
    types::{Callable, Comparable, Constructor, Hashable, PyComparisonOp, Representable},
};

pub use crate::object::PyWeak;

#[derive(FromArgs)]
pub struct WeakNewArgs {
    #[pyarg(positional)]
    referent: PyObjectRef,
    #[pyarg(positional, optional)]
    callback: OptionalArg<PyObjectRef>,
}

impl PyPayload for PyWeak {
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.weakref_type
    }
}

impl Callable for PyWeak {
    type Args = ();
    #[inline]
    fn call(zelf: &Py<Self>, _: Self::Args, vm: &VirtualMachine) -> PyResult {
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
        Ok(weak.into())
    }
}

#[pyclass(
    with(Callable, Hashable, Comparable, Constructor, Representable),
    flags(BASETYPE)
)]
impl PyWeak {
    #[pyclassmethod(magic)]
    fn class_getitem(cls: PyTypeRef, args: PyObjectRef, vm: &VirtualMachine) -> PyGenericAlias {
        PyGenericAlias::new(cls, args, vm)
    }
}

impl Hashable for PyWeak {
    fn hash(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyHash> {
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
        zelf: &Py<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<crate::function::PyComparisonValue> {
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

impl Representable for PyWeak {
    #[inline]
    fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
        let id = zelf.get_id();
        let repr = if let Some(o) = zelf.upgrade() {
            format!(
                "<weakref at {:#x}; to '{}' at {:#x}>",
                id,
                o.class().name(),
                o.get_id(),
            )
        } else {
            format!("<weakref at {id:#x}; dead>")
        };
        Ok(repr)
    }
}

pub fn init(context: &Context) {
    PyWeak::extend_class(context, context.types.weakref_type);
}
