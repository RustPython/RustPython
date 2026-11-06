use super::{
    PyGenericAlias, PyTupleRef, PyType, PyTypeRef,
    iter::{SequenceIter, SequenceIterDirection},
};
use crate::common::lock::PyRwLock;
use crate::{
    AsObject, Context, Py, PyObjectRef, PyPayload, PyResult, VirtualMachine,
    class::PyClassImpl,
    function::{ArgIndex, FuncArgs},
    protocol::{PyIter, PyIterReturn},
    raise_if_stop,
    types::{Constructor, IterNext, Iterable, SelfIter, drop_kwargs_if_init_overridden},
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
    pub(crate) fn from_object(
        cls: PyTypeRef,
        sequence: &PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult {
        // Keep the original sequence alive while binding, calling, and releasing
        // the special method, including when it does not retain its receiver.
        if let Some(method) = vm.get_method(sequence.clone(), identifier!(vm, __reversed__)) {
            let method = method?;
            if vm.is_none(&method) {
                return Err(Self::not_reversible(sequence, vm));
            }
            let result = method.call((), vm);
            return result;
        }

        Self::py_new(
            &cls,
            ReversedArgs {
                object: sequence.clone(),
            },
            vm,
        )?
        .into_ref_with_type(vm, cls)
        .map(Into::into)
    }

    fn not_reversible(
        sequence: &PyObjectRef,
        vm: &VirtualMachine,
    ) -> crate::builtins::PyBaseExceptionRef {
        vm.new_type_error(format!(
            "'{}' object is not reversible",
            sequence.class().slot_name()
        ))
    }

    pub(crate) const fn new(obj: PyObjectRef, len: usize) -> Self {
        let position = len as isize - 1;
        Self {
            internal: SequenceIter::new(obj, position, SequenceIterDirection::Reverse),
        }
    }
}

#[derive(FromArgs)]
pub(crate) struct ReversedArgs {
    #[pyarg(positional)]
    object: PyObjectRef,
}

impl Constructor for PyReverseSequenceIterator {
    type Args = ReversedArgs;

    fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        // A subtype's custom initializer may consume keywords, but the exact
        // type and subtypes inheriting its initializer reject them first.
        // Retain the original arguments through callbacks even if keywords
        // are removed from the constructor's view of the call.
        let constructor_args =
            drop_kwargs_if_init_overridden(&cls, vm.ctx.types.reverse_iter_type, args.clone());
        if !constructor_args.kwargs.is_empty() {
            return Err(vm.new_type_error("reversed() takes no keyword arguments"));
        }
        let ReversedArgs { object } = constructor_args.bind_for(vm, "reversed")?;
        let result = Self::from_object(cls, &object, vm);
        drop(object);
        drop(args);
        result
    }

    fn py_new(
        _cls: &Py<PyType>,
        Self::Args { object }: Self::Args,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        // The slot handles arbitrary __reversed__ results before constructing
        // a payload. Checking sequence support must not bind __getitem__.
        let protocol = object.sequence_unchecked();
        if !protocol.check() {
            return Err(Self::not_reversible(&object, vm));
        }
        let len = protocol.length(vm)?;
        Ok(Self::new(object, len))
    }
}

#[pyclass(with(IterNext, Iterable, Constructor), flags(BASETYPE))]
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
    fn __reduce__(&self, vm: &VirtualMachine) -> PyTupleRef {
        let func = self.class().to_owned().into();
        self.internal.reduce(func, vm)
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
