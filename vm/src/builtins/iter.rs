/*
 * iterator types
 */

use crossbeam_utils::atomic::AtomicCell;

use super::pytype::PyTypeRef;
use crate::slots::PyIter;
use crate::vm::VirtualMachine;
use crate::{
    ItemProtocol, PyCallable, PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue,
    TypeProtocol,
};

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
    pub position: AtomicCell<isize>,
    pub obj: PyObjectRef,
}

impl PyValue for PySequenceIterator {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.iter_type
    }
}

#[pyimpl(with(PyIter))]
impl PySequenceIterator {
    pub fn new_forward(obj: PyObjectRef) -> Self {
        Self {
            position: AtomicCell::new(0),
            obj,
        }
    }

    #[pymethod(magic)]
    fn length_hint(&self, vm: &VirtualMachine) -> PyResult<isize> {
        let pos = self.position.load();
        let len = vm.obj_len(&self.obj)?;
        Ok(len as isize - pos)
    }
}

impl PyIter for PySequenceIterator {
    fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        let pos = zelf.position.fetch_add(1);
        match zelf.obj.get_item(pos, vm) {
            Err(ref e) if e.isinstance(&vm.ctx.exceptions.index_error) => {
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
    callable: PyCallable,
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
    pub fn new(callable: PyCallable, sentinel: PyObjectRef) -> Self {
        Self {
            callable,
            sentinel,
            status: AtomicCell::new(IterStatus::Active),
        }
    }
}

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
