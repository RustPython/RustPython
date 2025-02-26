use super::{
    IterStatus, PositionIterInternal, PyGenericAlias, PyIntRef, PyTupleRef, PyType, PyTypeRef,
};
use crate::common::lock::{PyMutex, PyRwLock};
use crate::{
    AsObject, Context, Py, PyObjectRef, PyPayload, PyResult, VirtualMachine,
    class::PyClassImpl,
    convert::ToPyObject,
    function::OptionalArg,
    protocol::{PyIter, PyIterReturn},
    types::{Constructor, IterNext, Iterable, SelfIter},
};
use malachite_bigint::BigInt;
use num_traits::Zero;

#[pyclass(module = false, name = "enumerate", traverse)]
#[derive(Debug)]
pub struct PyEnumerate {
    #[pytraverse(skip)]
    counter: PyRwLock<BigInt>,
    iterator: PyIter,
}

impl PyPayload for PyEnumerate {
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.enumerate_type
    }
}

#[derive(FromArgs)]
pub struct EnumerateArgs {
    iterator: PyIter,
    #[pyarg(any, optional)]
    start: OptionalArg<PyIntRef>,
}

impl Constructor for PyEnumerate {
    type Args = EnumerateArgs;

    fn py_new(
        cls: PyTypeRef,
        Self::Args { iterator, start }: Self::Args,
        vm: &VirtualMachine,
    ) -> PyResult {
        let counter = start.map_or_else(BigInt::zero, |start| start.as_bigint().clone());
        PyEnumerate {
            counter: PyRwLock::new(counter),
            iterator,
        }
        .into_ref_with_type(vm, cls)
        .map(Into::into)
    }
}

#[pyclass(with(Py, IterNext, Iterable, Constructor), flags(BASETYPE))]
impl PyEnumerate {
    #[pyclassmethod(magic)]
    fn class_getitem(cls: PyTypeRef, args: PyObjectRef, vm: &VirtualMachine) -> PyGenericAlias {
        PyGenericAlias::new(cls, args, vm)
    }
}

#[pyclass]
impl Py<PyEnumerate> {
    #[pymethod(magic)]
    fn reduce(&self) -> (PyTypeRef, (PyIter, BigInt)) {
        (
            self.class().to_owned(),
            (self.iterator.clone(), self.counter.read().clone()),
        )
    }
}

impl SelfIter for PyEnumerate {}
impl IterNext for PyEnumerate {
    fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        let next_obj = match zelf.iterator.next(vm)? {
            PyIterReturn::StopIteration(v) => return Ok(PyIterReturn::StopIteration(v)),
            PyIterReturn::Return(obj) => obj,
        };
        let mut counter = zelf.counter.write();
        let position = counter.clone();
        *counter += 1;
        Ok(PyIterReturn::Return((position, next_obj).to_pyobject(vm)))
    }
}

#[pyclass(module = false, name = "reversed", traverse)]
#[derive(Debug)]
pub struct PyReverseSequenceIterator {
    internal: PyMutex<PositionIterInternal<PyObjectRef>>,
}

impl PyPayload for PyReverseSequenceIterator {
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.reverse_iter_type
    }
}

#[pyclass(with(IterNext, Iterable))]
impl PyReverseSequenceIterator {
    pub fn new(obj: PyObjectRef, len: usize) -> Self {
        let position = len.saturating_sub(1);
        Self {
            internal: PyMutex::new(PositionIterInternal::new(obj, position)),
        }
    }

    #[pymethod(magic)]
    fn length_hint(&self, vm: &VirtualMachine) -> PyResult<usize> {
        let internal = self.internal.lock();
        if let IterStatus::Active(obj) = &internal.status {
            if internal.position <= obj.length(vm)? {
                return Ok(internal.position + 1);
            }
        }
        Ok(0)
    }

    #[pymethod(magic)]
    fn setstate(&self, state: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.internal.lock().set_state(state, |_, pos| pos, vm)
    }

    #[pymethod(magic)]
    fn reduce(&self, vm: &VirtualMachine) -> PyTupleRef {
        self.internal
            .lock()
            .builtins_reversed_reduce(|x| x.clone(), vm)
    }
}

impl SelfIter for PyReverseSequenceIterator {}
impl IterNext for PyReverseSequenceIterator {
    fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        zelf.internal
            .lock()
            .rev_next(|obj, pos| PyIterReturn::from_getitem_result(obj.get_item(&pos, vm), vm))
    }
}

pub fn init(context: &Context) {
    PyEnumerate::extend_class(context, context.types.enumerate_type);
    PyReverseSequenceIterator::extend_class(context, context.types.reverse_iter_type);
}
