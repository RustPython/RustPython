/*
 * iterator types
 */

use super::{PyInt, PyTupleRef, PyType};
use crate::{
    Context, Py, PyObjectRef, PyPayload, PyResult, VirtualMachine,
    class::PyClassImpl,
    function::ArgCallable,
    object::{Traverse, TraverseFn},
    protocol::PyIterReturn,
    types::{IterNext, Iterable, SelfIter},
};
use rustpython_common::lock::{PyMutex, PyRwLock, PyRwLockUpgradableReadGuard};

/// Marks status of iterator.
#[derive(Debug, Clone)]
pub enum IterStatus<T> {
    /// Iterator hasn't raised StopIteration.
    Active(T),
    /// Iterator has raised StopIteration.
    Exhausted,
}

unsafe impl<T: Traverse> Traverse for IterStatus<T> {
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        match self {
            Self::Active(r) => r.traverse(tracer_fn),
            Self::Exhausted => (),
        }
    }
}

#[derive(Debug)]
pub struct PositionIterInternal<T> {
    pub status: IterStatus<T>,
    pub position: usize,
}

unsafe impl<T: Traverse> Traverse for PositionIterInternal<T> {
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        self.status.traverse(tracer_fn)
    }
}

impl<T> PositionIterInternal<T> {
    pub const fn new(obj: T, position: usize) -> Self {
        Self {
            status: IterStatus::Active(obj),
            position,
        }
    }

    pub fn set_state<F>(&mut self, state: PyObjectRef, f: F, vm: &VirtualMachine) -> PyResult<()>
    where
        F: FnOnce(&T, usize) -> usize,
    {
        if let IterStatus::Active(obj) = &self.status {
            if let Some(i) = state.downcast_ref::<PyInt>() {
                let i = i.try_to_primitive(vm).unwrap_or(0);
                self.position = f(obj, i);
                Ok(())
            } else {
                Err(vm.new_type_error("an integer is required."))
            }
        } else {
            Ok(())
        }
    }

    /// Build a pickle-compatible reduce tuple.
    ///
    /// `func` must be resolved **before** acquiring any lock that guards this
    /// `PositionIterInternal`, so that the builtins lookup cannot trigger
    /// reentrant iterator access and deadlock.
    pub fn reduce<F, E>(
        &self,
        func: PyObjectRef,
        active: F,
        empty: E,
        vm: &VirtualMachine,
    ) -> PyTupleRef
    where
        F: FnOnce(&T) -> PyObjectRef,
        E: FnOnce(&VirtualMachine) -> PyObjectRef,
    {
        if let IterStatus::Active(obj) = &self.status {
            vm.new_tuple((func, (active(obj),), self.position))
        } else {
            vm.new_tuple((func, (empty(vm),)))
        }
    }

    fn _next<F, OP>(&mut self, f: F, op: OP) -> PyResult<PyIterReturn>
    where
        F: FnOnce(&T, usize) -> PyResult<PyIterReturn>,
        OP: FnOnce(&mut Self),
    {
        if let IterStatus::Active(obj) = &self.status {
            let ret = f(obj, self.position);
            if let Ok(PyIterReturn::Return(_)) = ret {
                op(self);
            } else {
                self.status = IterStatus::Exhausted;
            }
            ret
        } else {
            Ok(PyIterReturn::StopIteration(None))
        }
    }

    pub fn next<F>(&mut self, f: F) -> PyResult<PyIterReturn>
    where
        F: FnOnce(&T, usize) -> PyResult<PyIterReturn>,
    {
        self._next(f, |zelf| zelf.position += 1)
    }

    pub fn rev_next<F>(&mut self, f: F) -> PyResult<PyIterReturn>
    where
        F: FnOnce(&T, usize) -> PyResult<PyIterReturn>,
    {
        self._next(f, |zelf| {
            if zelf.position == 0 {
                zelf.status = IterStatus::Exhausted;
            } else {
                zelf.position -= 1;
            }
        })
    }

    pub fn length_hint<F>(&self, f: F) -> usize
    where
        F: FnOnce(&T) -> usize,
    {
        if let IterStatus::Active(obj) = &self.status {
            f(obj).saturating_sub(self.position)
        } else {
            0
        }
    }

    pub fn rev_length_hint<F>(&self, f: F) -> usize
    where
        F: FnOnce(&T) -> usize,
    {
        if let IterStatus::Active(obj) = &self.status
            && self.position <= f(obj)
        {
            return self.position + 1;
        }
        0
    }
}

pub fn builtins_iter(vm: &VirtualMachine) -> PyObjectRef {
    vm.builtins.get_attr("iter", vm).unwrap()
}

pub fn builtins_reversed(vm: &VirtualMachine) -> PyObjectRef {
    vm.builtins.get_attr("reversed", vm).unwrap()
}

#[pyclass(module = false, name = "iterator", traverse)]
#[derive(Debug)]
pub struct PySequenceIterator {
    internal: PyMutex<PositionIterInternal<PyObjectRef>>,
}

impl PyPayload for PySequenceIterator {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.iter_type
    }
}

#[pyclass(with(IterNext, Iterable))]
impl PySequenceIterator {
    pub fn new(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<Self> {
        let _seq = obj.try_sequence(vm)?;
        Ok(Self {
            internal: PyMutex::new(PositionIterInternal::new(obj, 0)),
        })
    }

    #[pymethod]
    fn __length_hint__(&self, vm: &VirtualMachine) -> PyObjectRef {
        let internal = self.internal.lock();
        if let IterStatus::Active(obj) = &internal.status {
            let seq = obj.sequence_unchecked();
            seq.length(vm)
                .map(|x| PyInt::from(x).into_pyobject(vm))
                .unwrap_or_else(|_| vm.ctx.not_implemented())
        } else {
            PyInt::from(0).into_pyobject(vm)
        }
    }

    #[pymethod]
    fn __reduce__(&self, vm: &VirtualMachine) -> PyTupleRef {
        let func = builtins_iter(vm);
        self.internal.lock().reduce(
            func,
            |x| x.clone(),
            |vm| vm.ctx.empty_tuple.clone().into(),
            vm,
        )
    }

    #[pymethod]
    fn __setstate__(&self, state: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.internal.lock().set_state(state, |_, pos| pos, vm)
    }
}

impl SelfIter for PySequenceIterator {}
impl IterNext for PySequenceIterator {
    fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        zelf.internal.lock().next(|obj, pos| {
            let seq = obj.sequence_unchecked();
            PyIterReturn::from_getitem_result(seq.get_item(pos as isize, vm), vm)
        })
    }
}

#[pyclass(module = false, name = "callable_iterator", traverse)]
#[derive(Debug)]
pub struct PyCallableIterator {
    sentinel: PyObjectRef,
    status: PyRwLock<IterStatus<ArgCallable>>,
}

impl PyPayload for PyCallableIterator {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.callable_iterator
    }
}

#[pyclass(with(IterNext, Iterable))]
impl PyCallableIterator {
    pub const fn new(callable: ArgCallable, sentinel: PyObjectRef) -> Self {
        Self {
            sentinel,
            status: PyRwLock::new(IterStatus::Active(callable)),
        }
    }

    #[pymethod]
    fn __reduce__(&self, vm: &VirtualMachine) -> PyTupleRef {
        let func = builtins_iter(vm);
        let status = self.status.read();
        if let IterStatus::Active(callable) = &*status {
            let callable_obj: PyObjectRef = callable.clone().into();
            vm.new_tuple((func, (callable_obj, self.sentinel.clone())))
        } else {
            vm.new_tuple((func, (vm.ctx.empty_tuple.clone(),)))
        }
    }
}

impl SelfIter for PyCallableIterator {}
impl IterNext for PyCallableIterator {
    fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        // Clone the callable and release the lock before invoking,
        // so that reentrant next() calls don't deadlock.
        let callable = {
            let status = zelf.status.read();
            match &*status {
                IterStatus::Active(callable) => callable.clone(),
                IterStatus::Exhausted => return Ok(PyIterReturn::StopIteration(None)),
            }
        };

        let ret = callable.invoke((), vm)?;

        // Re-check: a reentrant call may have exhausted the iterator.
        let status = zelf.status.upgradable_read();
        if !matches!(&*status, IterStatus::Active(_)) {
            return Ok(PyIterReturn::StopIteration(None));
        }

        if vm.bool_eq(&ret, &zelf.sentinel)? {
            *PyRwLockUpgradableReadGuard::upgrade(status) = IterStatus::Exhausted;
            Ok(PyIterReturn::StopIteration(None))
        } else {
            Ok(PyIterReturn::Return(ret))
        }
    }
}

pub fn init(context: &Context) {
    PySequenceIterator::extend_class(context, context.types.iter_type);
    PyCallableIterator::extend_class(context, context.types.callable_iterator);
}
