use super::{PyGenericAlias, PyType, PyTypeRef};
use crate::common::{
    atomic::{Ordering, Radium},
    hash::{self, PyHash},
};
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    class::PyClassImpl,
    function::{FuncArgs, OptionalArg},
    types::{
        Callable, Comparable, Constructor, Hashable, Initializer, PyComparisonOp, Representable,
    },
};

pub use crate::object::PyWeak;

#[derive(FromArgs)]
#[allow(dead_code)]
pub struct WeakNewArgs {
    #[pyarg(positional)]
    referent: PyObjectRef,
    #[pyarg(positional, optional)]
    callback: OptionalArg<PyObjectRef>,
}

impl PyPayload for PyWeak {
    #[inline]
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

    fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        // PyArg_UnpackTuple: only process positional args, ignore kwargs.
        // Subclass __init__ will handle extra kwargs.
        let mut positional = args.args.into_iter();
        let referent = positional.next().ok_or_else(|| {
            vm.new_type_error("__new__ expected at least 1 argument, got 0".to_owned())
        })?;
        let callback = positional.next();
        if let Some(_extra) = positional.next() {
            return Err(vm.new_type_error(format!(
                "__new__ expected at most 2 arguments, got {}",
                3 + positional.count()
            )));
        }
        let weak = referent.downgrade_with_typ(callback, cls, vm)?;
        Ok(weak.into())
    }

    fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
        unimplemented!("use slot_new")
    }
}

impl Initializer for PyWeak {
    type Args = WeakNewArgs;

    // weakref_tp_init: accepts args but does nothing (all init done in slot_new)
    fn init(_zelf: PyRef<Self>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<()> {
        Ok(())
    }
}

#[pyclass(
    with(
        Callable,
        Hashable,
        Comparable,
        Constructor,
        Initializer,
        Representable
    ),
    flags(BASETYPE)
)]
impl PyWeak {
    #[pyclassmethod]
    fn __class_getitem__(cls: PyTypeRef, args: PyObjectRef, vm: &VirtualMachine) -> PyGenericAlias {
        PyGenericAlias::from_args(cls, args, vm)
    }
}

impl Hashable for PyWeak {
    fn hash(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyHash> {
        let hash = match zelf.hash.load(Ordering::Relaxed) {
            hash::SENTINEL => {
                let obj = zelf
                    .upgrade()
                    .ok_or_else(|| vm.new_type_error("weak object has gone away"))?;
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
