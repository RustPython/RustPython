use super::{
    PyGenericAlias, PyTupleRef, PyType, PyTypeRef,
    iter::{SequenceIter, SequenceIterDirection, builtins_reversed},
};
use crate::common::lock::PyRwLock;
use crate::{
    AsObject, Context, Py, PyObjectRef, PyPayload, PyResult, VirtualMachine,
    class::PyClassImpl,
    function::ArgIndex,
    protocol::{PyIter, PyIterReturn},
    raise_if_stop,
    types::{Constructor, IterNext, Iterable, SelfIter},
};
use malachite_bigint::BigInt;
use num_traits::ToPrimitive;

/// Fast-path counter for `enumerate`.
///
/// malachite `BigInt` already keeps limb-sized values inline (`Natural::Small`)
/// and only heap-allocates beyond that, so this enum looks redundant. The
/// small/large tag is crate-private, so we cannot inspect or bump that limb
/// ourselves; storing a `BigInt` would still clone it and go through
/// `+= 1` on every `next()`. Keep a `usize` until it overflows, or when
/// `start` does not fit in `usize`.
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
    // Bound as an object so the other arguments are checked before `iter()` runs, as in CPython.
    #[pyarg(any)]
    iterable: PyObjectRef,
    #[pyarg(any, default = 0)]
    start: ArgIndex,
}

impl Constructor for PyEnumerate {
    type Args = EnumerateArgs;

    fn py_new(
        _cls: &Py<PyType>,
        Self::Args { iterable, start }: Self::Args,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        let iterable = iterable.get_iter(vm)?;
        let start = start.into_int_ref();
        let counter = match start.as_bigint().to_usize() {
            Some(n) => Counter::Small(n),
            None => Counter::Big(start.as_bigint().clone()),
        };
        Ok(Self {
            counter: PyRwLock::new(counter),
            iterable,
        })
    }
}

#[pyclass(with(IterNext, Iterable, Constructor), flags(BASETYPE))]
impl Py<PyEnumerate> {
    #[pyclassmethod]
    fn __class_getitem__(
        cls: PyTypeRef,
        object: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyGenericAlias> {
        PyGenericAlias::from_args(cls, object, vm)
    }

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
    internal: SequenceIter,
}

impl PyPayload for PyReverseSequenceIterator {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.reverse_iter_type
    }
}

impl PyReverseSequenceIterator {
    pub(crate) const fn new(obj: PyObjectRef, len: usize) -> Self {
        let position = len as isize - 1;
        Self {
            internal: SequenceIter::new(obj, position, SequenceIterDirection::Reverse),
        }
    }
}

#[pyclass(with(IterNext, Iterable))]
impl Py<PyReverseSequenceIterator> {
    #[pymethod]
    fn __length_hint__(&self, vm: &VirtualMachine) -> PyResult<usize> {
        vm.with_recursion(" in __length_hint__", || {
            self.internal
                .length_hint(vm)
                .map(|length| length.unwrap_or(0))
        })
    }

    #[pymethod]
    fn __setstate__(&self, state: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.internal.set_state(&state, vm)
    }

    #[pymethod]
    fn __reduce__(&self, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
        let func = builtins_reversed(vm)?;
        Ok(self.internal.reduce(func, vm))
    }
}

impl SelfIter for PyReverseSequenceIterator {}
impl IterNext for PyReverseSequenceIterator {
    fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        zelf.internal.next(vm)
    }
}

pub(crate) fn init(context: &'static Context) {
    PyEnumerate::extend_class(context, context.types.enumerate_type);
    PyReverseSequenceIterator::extend_class(context, context.types.reverse_iter_type);
}
