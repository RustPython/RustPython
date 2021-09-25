use super::PyTypeRef;
use crate::common::hash::PyHash;
use crate::{
    function::{FuncArgs, OptionalArg},
    slots::{Callable, Comparable, Hashable, PyComparisonOp, SlotConstructor},
    IdProtocol, PyClassImpl, PyContext, PyObjectRef, PyObjectWeak, PyRef, PyResult, PyValue,
    TypeProtocol, VirtualMachine,
};

use crossbeam_utils::atomic::AtomicCell;

#[pyclass(module = false, name = "weakref")]
#[derive(Debug)]
pub struct PyWeak {
    referent: PyObjectWeak,
    hash: AtomicCell<Option<PyHash>>,
}

impl PyWeak {
    pub fn downgrade(obj: &PyObjectRef) -> PyWeak {
        PyWeak {
            referent: PyObjectRef::downgrade(obj),
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
    fn call(zelf: &PyRef<Self>, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        args.bind::<()>(vm)?;
        Ok(vm.unwrap_or_none(zelf.upgrade()))
    }
}

impl SlotConstructor for PyWeak {
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

#[pyimpl(with(Callable, Hashable, Comparable, SlotConstructor), flags(BASETYPE))]
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
}

impl Hashable for PyWeak {
    fn hash(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyHash> {
        match zelf.hash.load() {
            Some(hash) => Ok(hash),
            None => {
                let obj = zelf
                    .upgrade()
                    .ok_or_else(|| vm.new_type_error("weak object has gone away".to_owned()))?;
                let hash = vm._hash(&obj)?;
                zelf.hash.store(Some(hash));
                Ok(hash)
            }
        }
    }
}

impl Comparable for PyWeak {
    fn cmp(
        zelf: &PyRef<Self>,
        other: &PyObjectRef,
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
