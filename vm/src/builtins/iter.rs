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

#[derive(Debug)]
pub struct PositionIterInternal {
    pub position: AtomicCell<usize>,
    /// object or PyNone if exhausted
    pub obj: PyRwLock<PyObjectRef>,
}

impl PositionIterInternal {
    pub fn new(obj: PyObjectRef) -> Self {
        Self {
            position: AtomicCell::new(0),
            obj: PyRwLock::new(obj),
        }
    }

    pub fn is_active(&self, vm: &VirtualMachine) -> bool {
        !vm.is_none(&self.obj.read())
    }

    pub fn set_state(&self, state: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        if self.is_active(vm) {
            if let Some(i) = state.payload::<PyInt>() {
                let i = int::try_to_primitive(i.as_bigint(), vm).unwrap_or(0);
                self.position.store(i);
                Ok(())
            } else {
                Err(vm.new_type_error("an integer is required.".to_owned()))
            }
        } else {
            Ok(())
        }
    }

    pub fn reduce(&self, func: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if self.is_active(vm) {
            vm.ctx.new_tuple(vec![
                func,
                vm.ctx.new_tuple(vec![self.obj.read().clone()]),
                vm.ctx.new_int(self.position.load()),
            ])
        } else {
            vm.ctx
                .new_tuple(vec![func, vm.ctx.new_tuple(vec![vm.ctx.new_list(vec![])])])
        }
    }

    pub fn next<F>(&self, f: F, vm: &VirtualMachine) -> PyResult
    where
        F: FnOnce(usize) -> PyResult,
    {
        if self.is_active(vm) {
            let pos = self.position.fetch_add(1);
            match f(pos) {
                Err(ref e)
                    if e.isinstance(&vm.ctx.exceptions.index_error)
                        || e.isinstance(&vm.ctx.exceptions.stop_iteration) =>
                {
                    *self.obj.write() = vm.ctx.none();
                    Err(vm.new_stop_iteration())
                }
                ret => ret,
            }
        } else {
            Err(vm.new_stop_iteration())
        }
    }

    pub fn length_hint<F>(&self, f: F, vm: &VirtualMachine) -> PyObjectRef
    where
        F: FnOnce() -> Option<usize>,
    {
        let len = if self.is_active(vm) {
            let pos = self.position.load();
            if let Some(obj_len) = f() {
                obj_len.saturating_sub(pos)
            } else {
                return vm.ctx.not_implemented();
            }
        } else {
            0
        };
        PyInt::from(len).into_object(vm)
    }
}

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
    internal: PositionIterInternal,
    // pub position: AtomicCell<usize>,
    // pub obj: PyObjectRef,
    // pub status: AtomicCell<IterStatus>
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
            internal: PositionIterInternal::new(obj),
        }
    }

    #[pymethod(magic)]
    fn length_hint(&self, vm: &VirtualMachine) -> PyObjectRef {
        self.internal.length_hint(
            || {
                vm.obj_len(&self.internal.obj.read()).ok()
            },
            vm,
        )
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
        zelf.internal
            .next(|pos| zelf.internal.obj.read().get_item(pos, vm), vm)
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

#[pyimpl(with(SlotIterator))]
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
impl SlotIterator for PyCallableIterator {
    fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        if let IterStatus::Exhausted = zelf.status.load() {
            return Ok(PyIterReturn::StopIteration(None));
        }
        let ret = zelf.callable.invoke((), vm)?;
        if vm.bool_eq(&ret, &zelf.sentinel)? {
            zelf.status.store(IterStatus::Exhausted);
            Ok(PyIterReturn::StopIteration(None))
        } else {
            Ok(PyIterReturn::Return(ret))
        }
    }
}

pub fn init(context: &PyContext) {
    PySequenceIterator::extend_class(context, &context.types.iter_type);
    PyCallableIterator::extend_class(context, &context.types.callable_iterator);
}
