use super::{PyGenericAlias, PyType, PyTypeRef};
#[cfg(not(feature = "threading"))]
use crate::common::atomic::Radium;
use crate::common::{
    atomic::Ordering,
    hash::{self, PyHash},
};
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyResult, VirtualMachine,
    class::PyClassImpl,
    function::{Either, FuncArgs, OptionalArg, PyArithmeticValue, PyComparisonValue},
    types::{
        Callable, Comparable, Constructor, Hashable, Initializer, PyComparisonOp, Representable,
    },
};
use rustpython_common::wtf8::{Wtf8Buf, wtf8_concat};

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
    // WeakRefList::add tracks only after linking and publishing the referent.
    const NEW_REF_UNTRACKED: bool = true;

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
        let referent = positional
            .next()
            .ok_or_else(|| vm.new_arity_type_error("__new__", 1..=2, 0))?;
        let callback = positional.next().filter(|callback| !vm.is_none(callback));
        if let Some(_extra) = positional.next() {
            let got = positional.count() + 3;
            return Err(vm.new_arity_type_error("__new__", 1..=2, got));
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
    fn init(_zelf: &Py<Self>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<()> {
        Ok(())
    }
}

impl Comparable for PyWeak {
    fn slot_richcompare(
        zelf: &PyObject,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<Either<PyObjectRef, PyComparisonValue>> {
        if !matches!(op, PyComparisonOp::Eq | PyComparisonOp::Ne) {
            return Ok(Either::B(PyComparisonValue::NotImplemented));
        }
        // PyWeakref_Check also accepts both proxy types, which share this payload.
        let (Some(zelf), Some(other)) = (zelf.downcast_ref::<Self>(), other.downcast_ref::<Self>())
        else {
            return Ok(Either::B(PyComparisonValue::NotImplemented));
        };
        let a = zelf.upgrade();
        let b = other.upgrade();
        let result = match (&a, &b) {
            // PyObject_RichCompare preserves the exact operation and raw result,
            // including when both references point to the same object.
            (Some(a), Some(b)) => PyObject::rich_compare(a, b, op, vm).map(Either::A),
            _ => {
                let equal = zelf.is(other);
                Ok(Either::B((equal == (op == PyComparisonOp::Eq)).into()))
            }
        };
        // Release the upgraded referents in argument order, also on errors.
        drop(a);
        drop(b);
        result
    }

    fn cmp(
        zelf: &Py<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        match Self::slot_richcompare(zelf.as_object(), other, op, vm)? {
            Either::A(value) => PyArithmeticValue::from_object(vm, value)
                .map(|value| value.try_to_bool(vm))
                .transpose(),
            Either::B(value) => Ok(value),
        }
    }
}

#[pyclass(
    with(
        Callable,
        Comparable,
        Hashable,
        Constructor,
        Initializer,
        Representable
    ),
    flags(BASETYPE)
)]
impl Py<PyWeak> {
    #[pygetset]
    fn __callback__(&self, vm: &VirtualMachine) -> PyObjectRef {
        vm.unwrap_or_none(self.get_callback())
    }

    #[pyclassmethod]
    fn __class_getitem__(
        cls: PyTypeRef,
        object: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyGenericAlias> {
        PyGenericAlias::from_args(cls, object, vm)
    }
}

impl Hashable for PyWeak {
    fn hash(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyHash> {
        let cached = zelf.hash.load(Ordering::Relaxed);
        if cached != hash::SENTINEL {
            return Ok(cached);
        }
        let obj = zelf
            .upgrade()
            .ok_or_else(|| vm.new_type_error("weak object has gone away"))?;
        let result = obj.hash(vm).map(hash::fix_sentinel);
        // A recursive hash may have filled the cache. The outer call still
        // replaces it, or clears it on failure, before releasing the referent.
        zelf.hash.store(
            result.as_ref().copied().unwrap_or(hash::SENTINEL),
            Ordering::Relaxed,
        );
        drop(obj);
        result
    }
}

/// `__name__` when that lookup yields a string. A missing name or a non-string
/// value is left off the repr.
fn instance_name(obj: &PyObject, vm: &VirtualMachine) -> PyResult<Option<Wtf8Buf>> {
    let found =
        crate::vm::PyMethod::get_special_ex::<false>(obj, identifier!(vm, __name__), vm, true)?;
    let Some(crate::vm::PyMethod::Attribute(attr)) = found else {
        return Ok(None);
    };
    Ok(attr
        .downcast_ref::<crate::builtins::PyStr>()
        .map(|name| name.as_wtf8().to_owned()))
}

impl Representable for PyWeak {
    #[inline]
    fn repr_wtf8(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<Wtf8Buf> {
        let id = zelf.get_id();
        let Some(obj) = zelf.upgrade() else {
            return Ok(format!("<weakref at {id:#x}; dead>").into());
        };
        let type_name = obj.class().fully_qualified_name(vm)?;
        let obj_id = obj.get_id();
        let suffix = match instance_name(&obj, vm)? {
            Some(name) => wtf8_concat!(" (", name, ")"),
            None => Wtf8Buf::new(),
        };
        Ok(wtf8_concat!(
            format!("<weakref at {id:#x}; to '"),
            type_name,
            format!("' at {obj_id:#x}"),
            suffix,
            ">"
        ))
    }
}

pub(crate) fn init(context: &'static Context) {
    PyWeak::extend_class(context, context.types.weakref_type);
}
