use super::{IterStatus, PositionIterInternal, PyGenericAlias, PyIntRef, PyTupleRef, PyTypeRef};
use crate::common::lock::{PyMutex, PyRwLock};
use crate::{
    function::{IntoPyObject, OptionalArg},
    protocol::{PyIter, PyIterReturn},
    types::{Constructor, IterNext, IterNextIterable},
    ItemProtocol, PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue, TypeProtocol,
    VirtualMachine,
};
use num_bigint::BigInt;
use num_traits::Zero;

#[pyclass(module = false, name = "enumerate")]
#[derive(Debug)]
pub struct PyEnumerate {
    counter: PyRwLock<BigInt>,
    iterator: PyIter,
}

impl PyValue for PyEnumerate {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.enumerate_type
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
        .into_pyresult_with_type(vm, cls)
    }
}

#[pyimpl(with(IterNext, Constructor), flags(BASETYPE))]
impl PyEnumerate {
    #[pyclassmethod(magic)]
    fn class_getitem(cls: PyTypeRef, args: PyObjectRef, vm: &VirtualMachine) -> PyGenericAlias {
        PyGenericAlias::new(cls, args, vm)
    }
    #[pymethod(magic)]
    fn reduce(zelf: PyRef<Self>) -> (PyTypeRef, (PyIter, BigInt)) {
        (
            zelf.clone_class(),
            (zelf.iterator.clone(), zelf.counter.read().clone()),
        )
    }
}

impl IterNextIterable for PyEnumerate {}
impl IterNext for PyEnumerate {
    fn next(zelf: &crate::PyObjectView<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        let next_obj = match zelf.iterator.next(vm)? {
            PyIterReturn::StopIteration(v) => return Ok(PyIterReturn::StopIteration(v)),
            PyIterReturn::Return(obj) => obj,
        };
        let mut counter = zelf.counter.write();
        let position = counter.clone();
        *counter += 1;
        Ok(PyIterReturn::Return((position, next_obj).into_pyobject(vm)))
    }
}

#[pyclass(module = false, name = "reversed")]
#[derive(Debug)]
pub struct PyReverseSequenceIterator {
    internal: PyMutex<PositionIterInternal<PyObjectRef>>,
}

impl PyValue for PyReverseSequenceIterator {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.reverse_iter_type
    }
}

#[pyimpl(with(IterNext))]
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

impl IterNextIterable for PyReverseSequenceIterator {}
impl IterNext for PyReverseSequenceIterator {
    fn next(zelf: &crate::PyObjectView<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        zelf.internal
            .lock()
            .rev_next(|obj, pos| PyIterReturn::from_getitem_result(obj.get_item(pos, vm), vm))
    }
}

pub fn init(context: &PyContext) {
    PyEnumerate::extend_class(context, &context.types.enumerate_type);
    PyReverseSequenceIterator::extend_class(context, &context.types.reverse_iter_type);
}
