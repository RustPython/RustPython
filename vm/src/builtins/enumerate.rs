use super::{IterStatus, PositionIterInternal, PyIntRef, PyTypeRef};
use crate::common::lock::PyRwLock;
use crate::{
    function::OptionalArg,
    protocol::{PyIter, PyIterReturn},
    slots::{IteratorIterable, SlotConstructor, SlotIterator},
    IntoPyObject, ItemProtocol, PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue,
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

impl SlotConstructor for PyEnumerate {
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

#[pyimpl(with(SlotIterator, SlotConstructor), flags(BASETYPE))]
impl PyEnumerate {}

impl IteratorIterable for PyEnumerate {}
impl SlotIterator for PyEnumerate {
    fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
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
    internal: PyRwLock<PositionIterInternal<PyObjectRef>>,
}

impl PyValue for PyReverseSequenceIterator {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.reverse_iter_type
    }
}

#[pyimpl(with(SlotIterator))]
impl PyReverseSequenceIterator {
    pub fn new(obj: PyObjectRef, len: usize) -> Self {
        let position = len.saturating_sub(1);
        Self {
            internal: PyRwLock::new(PositionIterInternal::new(obj, position)),
        }
    }

    #[pymethod(magic)]
    fn length_hint(&self, vm: &VirtualMachine) -> PyResult<usize> {
        let internal = self.internal.read();
        if let IterStatus::Active(obj) = &internal.status {
            if internal.position <= vm.obj_len(obj)? {
                return Ok(internal.position + 1);
            }
        }
        Ok(0)
    }

    #[pymethod(magic)]
    fn setstate(&self, state: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.internal.write().set_state(state, vm)
    }

    #[pymethod(magic)]
    fn reduce(&self, vm: &VirtualMachine) -> PyObjectRef {
        self.internal
            .read()
            .builtin_reversed_reduce(|x| x.clone(), vm)
    }
}

impl IteratorIterable for PyReverseSequenceIterator {}
impl SlotIterator for PyReverseSequenceIterator {
    fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        zelf.internal
            .write()
            .rev_next(|obj, pos| obj.get_item(pos, vm), vm)
    }
}

pub fn init(context: &PyContext) {
    PyEnumerate::extend_class(context, &context.types.enumerate_type);
    PyReverseSequenceIterator::extend_class(context, &context.types.reverse_iter_type);
}
