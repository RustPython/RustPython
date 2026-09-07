use super::{
    IterStatus, PositionIterInternal, PyGenericAlias, PyIntRef, PyTupleRef, PyType, PyTypeRef,
    iter::builtins_reversed, locked_rev_next,
};
use crate::common::lock::{PyMutex, PyRwLock};
use crate::{
    AsObject, Context, Py, PyObjectRef, PyPayload, PyResult, VirtualMachine,
    class::PyClassImpl,
    function::OptionalArg,
    protocol::{PyIter, PyIterReturn},
    raise_if_stop,
    types::{Constructor, IterNext, Iterable, SelfIter},
};
use malachite_bigint::BigInt;
use num_traits::ToPrimitive;

/// Fast-path counter for `enumerate`. Most enumerations never exceed
/// `usize::MAX` iterations, so we keep the counter as a machine integer and
/// only fall back to arbitrary-precision arithmetic (matching CPython's
/// unbounded `count()`-style semantics) once it would overflow, or when the
/// caller supplied a `start` that doesn't fit in a `usize` to begin with.
#[derive(Debug, Clone)]
enum Counter {
    Small(usize),
    Big(BigInt),
}

impl Counter {
    fn to_bigint(&self) -> BigInt {
        match self {
            Self::Small(n) => BigInt::from(*n),
            Self::Big(b) => b.clone(),
        }
    }
}

#[pyclass(module = false, name = "enumerate", traverse)]
#[derive(Debug)]
pub struct PyEnumerate {
    #[pytraverse(skip)]
    counter: PyRwLock<Counter>,
    iterable: PyIter,
}

impl PyPayload for PyEnumerate {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.enumerate_type
    }
}

#[derive(FromArgs)]
pub struct EnumerateArgs {
    #[pyarg(any)]
    iterable: PyIter,
    #[pyarg(any, optional)]
    start: OptionalArg<PyIntRef>,
}

impl Constructor for PyEnumerate {
    type Args = EnumerateArgs;

    fn py_new(
        _cls: &Py<PyType>,
        Self::Args { iterable, start }: Self::Args,
        _vm: &VirtualMachine,
    ) -> PyResult<Self> {
        let counter = match start {
            OptionalArg::Present(start) => match start.as_bigint().to_usize() {
                Some(n) => Counter::Small(n),
                None => Counter::Big(start.as_bigint().clone()),
            },
            OptionalArg::Missing => Counter::Small(0),
        };
        Ok(Self {
            counter: PyRwLock::new(counter),
            iterable,
        })
    }
}

#[pyclass(with(Py, IterNext, Iterable, Constructor), flags(BASETYPE))]
impl PyEnumerate {
    #[pyclassmethod]
    fn __class_getitem__(
        cls: PyTypeRef,
        args: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyGenericAlias> {
        PyGenericAlias::from_args(cls, args, vm)
    }
}

#[pyclass]
impl Py<PyEnumerate> {
    #[pymethod]
    fn __reduce__(&self) -> (PyTypeRef, (PyIter, BigInt)) {
        (
            self.class().to_owned(),
            (self.iterable.clone(), self.counter.read().to_bigint()),
        )
    }
}

impl SelfIter for PyEnumerate {}

impl IterNext for PyEnumerate {
    fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        let next_obj = raise_if_stop!(zelf.iterable.next(vm)?);
        let mut counter = zelf.counter.write();
        let position = match &mut *counter {
            Counter::Small(n) => {
                let cur = *n;
                match cur.checked_add(1) {
                    Some(next_n) => {
                        *n = next_n;
                        vm.ctx.new_int(cur)
                    }
                    None => {
                        // Overflowed usize::MAX: promote to arbitrary precision,
                        // matching CPython's unbounded enumerate() semantics.
                        let cur_int = vm.ctx.new_int(cur);
                        *counter = Counter::Big(BigInt::from(cur) + 1);
                        cur_int
                    }
                }
            }
            Counter::Big(b) => {
                let position = b.clone();
                *b += 1;
                vm.ctx.new_bigint(&position)
            }
        };
        drop(counter);
        Ok(PyIterReturn::Return(
            vm.new_tuple((position, next_obj)).into(),
        ))
    }
}

#[pyclass(module = false, name = "reversed", traverse)]
#[derive(Debug)]
pub(crate) struct PyReverseSequenceIterator {
    internal: PyMutex<PositionIterInternal<PyObjectRef>>,
}

impl PyPayload for PyReverseSequenceIterator {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.reverse_iter_type
    }
}

#[pyclass(with(IterNext, Iterable))]
impl PyReverseSequenceIterator {
    pub(crate) const fn new(obj: PyObjectRef, len: usize) -> Self {
        let position = len.saturating_sub(1);
        Self {
            internal: PyMutex::new(PositionIterInternal::new(obj, position)),
        }
    }

    #[pymethod]
    fn __length_hint__(&self, vm: &VirtualMachine) -> PyResult<usize> {
        let internal = self.internal.lock();
        if let IterStatus::Active(obj) = &internal.status
            && internal.position <= obj.length(vm)?
        {
            return Ok(internal.position + 1);
        }
        Ok(0)
    }

    #[pymethod]
    fn __setstate__(&self, state: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.internal.lock().set_state(state, |_, pos| pos, vm)
    }

    #[pymethod]
    fn __reduce__(&self, vm: &VirtualMachine) -> PyTupleRef {
        let func = builtins_reversed(vm);
        self.internal.lock().reduce(
            func,
            |x| x.clone(),
            |vm| vm.ctx.empty_tuple.clone().into(),
            vm,
        )
    }
}

impl SelfIter for PyReverseSequenceIterator {}
impl IterNext for PyReverseSequenceIterator {
    fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        locked_rev_next(&zelf.internal, |obj, pos| {
            PyIterReturn::from_getitem_result(obj.get_item(&pos, vm), vm)
        })
    }
}

pub(crate) fn init(context: &'static Context) {
    PyEnumerate::extend_class(context, context.types.enumerate_type);
    PyReverseSequenceIterator::extend_class(context, context.types.reverse_iter_type);
}
