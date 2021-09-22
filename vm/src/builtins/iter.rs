/*
 * iterator types
 */

use super::{int, PyInt, PyTypeRef};
use crate::{
    function::ArgCallable,
    protocol::PyIterReturn,
    slots::{IteratorIterable, SlotIterator},
    ItemProtocol, PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue, TypeProtocol,
    VirtualMachine,
};
use crossbeam_utils::atomic::AtomicCell;

/// Marks status of iterator.
#[derive(Debug, Clone)]
pub enum IterStatus {
    /// Iterator hasn't raised StopIteration.
    Active(PyObjectRef),
    /// Iterator has raised StopIteration.
    Exhausted,
}

#[derive(Debug)]
pub struct PositionIterInternal {
    pub status: IterStatus,
    pub position: usize,
}

impl PositionIterInternal {
    pub fn new(obj: PyObjectRef, position: usize) -> Self {
        Self {
            status: IterStatus::Active(obj),
            position,
        }
    }

    pub fn set_state(&mut self, state: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        if let IterStatus::Active(_) = &self.status {
            if let Some(i) = state.payload::<PyInt>() {
                let i = int::try_to_primitive(i.as_bigint(), vm).unwrap_or(0);
                self.position = i;
                Ok(())
            } else {
                Err(vm.new_type_error("an integer is required.".to_owned()))
            }
        } else {
            Ok(())
        }
    }

    pub fn reduce(&self, func: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if let IterStatus::Active(obj) = &self.status {
            vm.ctx.new_tuple(vec![
                func,
                vm.ctx.new_tuple(vec![obj.clone()]),
                vm.ctx.new_int(self.position),
            ])
        } else {
            vm.ctx
                .new_tuple(vec![func, vm.ctx.new_tuple(vec![vm.ctx.new_list(vec![])])])
        }
    }

    pub fn next<F>(&mut self, f: F, vm: &VirtualMachine) -> PyResult
    where
        F: FnOnce(&PyObjectRef, usize) -> PyResult,
    {
        if let IterStatus::Active(obj) = &self.status {
            match f(obj, self.position) {
                Err(e) if e.isinstance(&vm.ctx.exceptions.stop_iteration) => {
                    self.status = IterStatus::Exhausted;
                    Err(e)
                }
                Err(e) if e.isinstance(&vm.ctx.exceptions.index_error) => {
                    self.status = IterStatus::Exhausted;
                    Err(vm.new_stop_iteration())
                }
                Err(e) => Err(e),
                Ok(ret) => {
                    self.position += 1;
                    Ok(ret)
                }
            }
        } else {
            Err(vm.new_stop_iteration())
        }
    }

    pub fn rev_next<F>(&mut self, f: F, vm: &VirtualMachine) -> PyResult
    where
        F: FnOnce(&PyObjectRef, usize) -> PyResult,
    {
        if let IterStatus::Active(obj) = &self.status {
            match f(obj, self.position) {
                Err(e) if e.isinstance(&vm.ctx.exceptions.stop_iteration) => {
                    self.status = IterStatus::Exhausted;
                    Err(e)
                }
                Err(e) if e.isinstance(&vm.ctx.exceptions.index_error) => {
                    self.status = IterStatus::Exhausted;
                    Err(vm.new_stop_iteration())
                }
                Err(e) => Err(e),
                Ok(ret) => {
                    if self.position == 0 {
                        self.status = IterStatus::Exhausted;
                    } else {
                        self.position -= 1;
                    }
                    Ok(ret)
                }
            }
        } else {
            Err(vm.new_stop_iteration())
        }
    }

    pub fn length_hint<F>(&self, f: F, vm: &VirtualMachine) -> PyObjectRef
    where
        F: FnOnce(&PyObjectRef) -> Option<usize>,
    {
        let len = if let IterStatus::Active(obj) = &self.status {
            if let Some(obj_len) = f(obj) {
                obj_len.saturating_sub(self.position)
            } else {
                return vm.ctx.not_implemented();
            }
        } else {
            0
        };
        PyInt::from(len).into_object(vm)
    }

    pub fn rev_length_hint<F>(&self, f: F, vm: &VirtualMachine) -> PyObjectRef
    where
        F: FnOnce(&PyObjectRef) -> Option<usize>,
    {
        if let IterStatus::Active(obj) = &self.status {
            if let Some(obj_len) = f(obj) {
                if self.position <= obj_len {
                    return PyInt::from(self.position + 1).into_object(vm);
                }
            }
        }
        PyInt::from(0).into_object(vm)
    }
}

#[pyclass(module = false, name = "iterator")]
#[derive(Debug)]
pub struct PySequenceIterator {
    internal: PositionIterInternal,
}

impl PyValue for PySequenceIterator {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.iter_type
    }
}

#[pyimpl(with(SlotIterator))]
impl PySequenceIterator {
    pub fn new(obj: PyObjectRef) -> Self {
        Self {
            internal: PositionIterInternal::new(obj, 0),
        }
    }

    #[pymethod(magic)]
    fn length_hint(&self, vm: &VirtualMachine) -> PyObjectRef {
        self.internal.length_hint(|obj| vm.obj_len(obj).ok(), vm)
    }

    #[pymethod(magic)]
    fn reduce(&self, vm: &VirtualMachine) -> PyResult {
        let iter = vm.get_attribute(vm.builtins.clone(), "iter")?;
        Ok(self.internal.reduce(iter, vm))
    }

    #[pymethod(magic)]
    fn setstate(&self, state: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.internal.set_state(state, vm)
    }
}

impl IteratorIterable for PySequenceIterator {}
impl SlotIterator for PySequenceIterator {
    fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        zelf.internal.next(|obj, pos| obj.get_item(pos, vm), vm)
    }
}

#[pyclass(module = false, name = "callable_iterator")]
#[derive(Debug)]
pub struct PyCallableIterator {
    sentinel: PyObjectRef,
    status: PyRwLock<IterStatus>,
}

impl PyValue for PyCallableIterator {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.callable_iterator
    }
}

#[pyimpl(with(SlotIterator))]
impl PyCallableIterator {
    pub fn new(callable: ArgCallable, sentinel: PyObjectRef) -> Self {
        Self {
            sentinel,
            status: PyRwLock::new(IterStatus::Active(callable.into_object())),
        }
    }
}

impl IteratorIterable for PyCallableIterator {}
impl SlotIterator for PyCallableIterator {
    fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        if let IterStatus::Active(callable) = &*zelf.status.read() {
            let ret = vm.invoke(callable, ())?;
            if vm.bool_eq(&ret, &zelf.sentinel)? {
                *zelf.status.write() = IterStatus::Exhausted;
                Err(vm.new_stop_iteration())
            } else {
                Ok(ret)
            }
        } else {
            Err(vm.new_stop_iteration())
        }
    }
}

pub fn init(context: &PyContext) {
    PySequenceIterator::extend_class(context, &context.types.iter_type);
    PyCallableIterator::extend_class(context, &context.types.callable_iterator);
}
