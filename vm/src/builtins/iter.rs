/*
 * iterator types
 */

use super::{int, PyInt, PyTypeRef};
use crate::{
    function::ArgCallable,
    slots::{IteratorIterable, PyIter},
    ItemProtocol, PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue, TypeProtocol,
    VirtualMachine,
};
use crossbeam_utils::atomic::AtomicCell;

/// Marks status of iterator.
#[derive(Debug, Clone, Copy)]
pub enum IterStatus {
    /// Iterator hasn't raised StopIteration.
    Active,
    /// Iterator has raised StopIteration.
    Exhausted,
}

#[pyclass(module = false, name = "iterator")]
#[derive(Debug)]
pub struct PySequenceIterator {
    pub position: AtomicCell<usize>,
    pub obj: PyObjectRef,
    pub status: AtomicCell<IterStatus>,
}

impl PyValue for PySequenceIterator {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.iter_type
    }
}

#[pyimpl(with(PyIter))]
impl PySequenceIterator {
    pub fn new(obj: PyObjectRef) -> Self {
        Self {
            position: AtomicCell::new(0),
            obj,
            status: AtomicCell::new(IterStatus::Active),
        }
    }

    #[pymethod(magic)]
    fn length_hint(&self, vm: &VirtualMachine) -> PyObjectRef {
        match self.status.load() {
            IterStatus::Active => {
                let pos = self.position.load();
                // return NotImplemented if no length is around.
                vm.obj_len(&self.obj)
                    .map_or(vm.ctx.not_implemented(), |len| {
                        PyInt::from(len.saturating_sub(pos)).into_object(vm)
                    })
            }
            IterStatus::Exhausted => PyInt::from(0).into_object(vm),
        }
    }

    #[pymethod(magic)]
    fn reduce(&self, vm: &VirtualMachine) -> PyResult {
        let iter = vm.get_attribute(vm.builtins.clone(), "iter")?;
        Ok(match self.status.load() {
            IterStatus::Exhausted => vm
                .ctx
                .new_tuple(vec![iter, vm.ctx.new_tuple(vec![vm.ctx.new_list(vec![])])]),
            IterStatus::Active => vm.ctx.new_tuple(vec![
                iter,
                vm.ctx.new_tuple(vec![self.obj.clone()]),
                vm.ctx.new_int(self.position.load()),
            ]),
        })
    }

    #[pymethod(magic)]
    fn setstate(&self, state: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        // When we're exhausted, just return.
        if let IterStatus::Exhausted = self.status.load() {
            return Ok(());
        }
        if let Some(i) = state.payload::<PyInt>() {
            self.position
                .store(int::try_to_primitive(i.as_bigint(), vm).unwrap_or(0));
            Ok(())
        } else {
            Err(vm.new_type_error("an integer is required.".to_owned()))
        }
    }
}

impl IteratorIterable for PySequenceIterator {}
impl PyIter for PySequenceIterator {
    fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        if let IterStatus::Exhausted = zelf.status.load() {
            return Err(vm.new_stop_iteration());
        }
        let pos = zelf.position.fetch_add(1);
        match zelf.obj.get_item(pos, vm) {
            Err(ref e) if e.isinstance(&vm.ctx.exceptions.index_error) => {
                zelf.status.store(IterStatus::Exhausted);
                Err(vm.new_stop_iteration())
            }
            // also catches stop_iteration => stop_iteration
            ret => ret,
        }
    }
}

#[pyclass(module = false, name = "callable_iterator")]
#[derive(Debug)]
pub struct PyCallableIterator {
    callable: ArgCallable,
    sentinel: PyObjectRef,
    status: AtomicCell<IterStatus>,
}

impl PyValue for PyCallableIterator {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.callable_iterator
    }
}

#[pyimpl(with(PyIter))]
impl PyCallableIterator {
    pub fn new(callable: ArgCallable, sentinel: PyObjectRef) -> Self {
        Self {
            callable,
            sentinel,
            status: AtomicCell::new(IterStatus::Active),
        }
    }
}

impl IteratorIterable for PyCallableIterator {}
impl PyIter for PyCallableIterator {
    fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        if let IterStatus::Exhausted = zelf.status.load() {
            return Err(vm.new_stop_iteration());
        }
        let ret = zelf.callable.invoke((), vm)?;
        if vm.bool_eq(&ret, &zelf.sentinel)? {
            zelf.status.store(IterStatus::Exhausted);
            Err(vm.new_stop_iteration())
        } else {
            Ok(ret)
        }
    }
}

pub fn init(context: &PyContext) {
    PySequenceIterator::extend_class(context, &context.types.iter_type);
    PyCallableIterator::extend_class(context, &context.types.callable_iterator);
}
