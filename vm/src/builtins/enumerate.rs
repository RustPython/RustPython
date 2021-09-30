use super::{
    int,
    IterStatus::{self, Active, Exhausted},
    PyInt, PyIntRef, PyTypeRef,
};
use crate::common::lock::PyRwLock;
use crate::{
    function::OptionalArg,
    protocol::{PyIter, PyIterReturn},
    slots::{IteratorIterable, SlotConstructor, SlotIterator},
    IntoPyObject, ItemProtocol, PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue,
    TypeProtocol, VirtualMachine,
};
use crossbeam_utils::atomic::AtomicCell;
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
    pub position: AtomicCell<usize>,
    pub status: AtomicCell<IterStatus>,
    pub obj: PyObjectRef,
}

impl PyValue for PyReverseSequenceIterator {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.reverse_iter_type
    }
}

#[pyimpl(with(SlotIterator))]
impl PyReverseSequenceIterator {
    pub fn new(obj: PyObjectRef, len: usize) -> Self {
        Self {
            position: AtomicCell::new(len.saturating_sub(1)),
            status: AtomicCell::new(if len == 0 { Exhausted } else { Active }),
            obj,
        }
    }

    #[pymethod(magic)]
    fn length_hint(&self, vm: &VirtualMachine) -> PyResult<usize> {
        Ok(match self.status.load() {
            Active => {
                let position = self.position.load();
                if position > vm.obj_len(&self.obj)? {
                    0
                } else {
                    position + 1
                }
            }
            Exhausted => 0,
        })
    }

    #[pymethod(magic)]
    fn setstate(&self, state: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        // When we're exhausted, just return.
        if let Exhausted = self.status.load() {
            return Ok(());
        }
        let len = vm.obj_len(&self.obj)?;
        let pos = state
            .payload::<PyInt>()
            .ok_or_else(|| vm.new_type_error("an integer is required.".to_owned()))?;
        let pos = std::cmp::min(
            int::try_to_primitive(pos.as_bigint(), vm).unwrap_or(0),
            len.saturating_sub(1),
        );
        self.position.store(pos);
        Ok(())
    }

    #[pymethod(magic)]
    fn reduce(&self, vm: &VirtualMachine) -> PyResult {
        let iter = vm.get_attribute(vm.builtins.clone(), "reversed")?;
        Ok(vm.ctx.new_tuple(match self.status.load() {
            Exhausted => vec![iter, vm.ctx.new_tuple(vec![vm.ctx.new_tuple(vec![])])],
            Active => vec![
                iter,
                vm.ctx.new_tuple(vec![self.obj.clone()]),
                vm.ctx.new_int(self.position.load()),
            ],
        }))
    }
}

impl IteratorIterable for PyReverseSequenceIterator {}
impl SlotIterator for PyReverseSequenceIterator {
    fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        if let Exhausted = zelf.status.load() {
            return Ok(PyIterReturn::StopIteration(None));
        }
        let pos = zelf.position.fetch_sub(1);
        if pos == 0 {
            zelf.status.store(Exhausted);
        }
        match zelf.obj.get_item(pos, vm) {
            Err(ref e)
                if e.isinstance(&vm.ctx.exceptions.index_error)
                    || e.isinstance(&vm.ctx.exceptions.stop_iteration) =>
            {
                zelf.status.store(Exhausted);
                Ok(PyIterReturn::StopIteration(None))
            }
            other => other.map(PyIterReturn::Return),
        }
    }
}

pub fn init(context: &PyContext) {
    PyEnumerate::extend_class(context, &context.types.enumerate_type);
    PyReverseSequenceIterator::extend_class(context, &context.types.reverse_iter_type);
}
