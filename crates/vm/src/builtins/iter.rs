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

    /// `op` answers whether the step it took left this exhausted.
    fn _next<F, OP>(&mut self, f: F, op: OP) -> (PyResult<PyIterReturn>, Option<T>)
    where
        F: FnOnce(&T, usize) -> PyResult<PyIterReturn>,
        OP: FnOnce(&mut Self) -> bool,
    {
        let IterStatus::Active(obj) = &self.status else {
            return (Ok(PyIterReturn::StopIteration(None)), None);
        };
        let ret = f(obj, self.position);
        let done = if let Ok(PyIterReturn::Return(_)) = ret {
            op(self)
        } else {
            true
        };
        let released = if done { self.exhaust() } else { None };
        (ret, released)
    }

    /// Mark this exhausted and hand back what it was holding, for the caller to
    /// release once it has dropped the lock guarding this. Releasing it under
    /// that lock would let a `__del__` that iterates again deadlock.
    #[must_use]
    pub fn exhaust(&mut self) -> Option<T> {
        match core::mem::replace(&mut self.status, IterStatus::Exhausted) {
            IterStatus::Active(obj) => Some(obj),
            IterStatus::Exhausted => None,
        }
    }

    /// Advance, along with what this was holding if the step exhausted it. See
    /// [`Self::exhaust`] for why the caller is handed it rather than the drop
    /// happening here; [`locked_next`] does the release for the common case.
    #[must_use = "what this hands back is released after the lock, not here"]
    pub fn next<F>(&mut self, f: F) -> (PyResult<PyIterReturn>, Option<T>)
    where
        F: FnOnce(&T, usize) -> PyResult<PyIterReturn>,
    {
        self._next(f, |zelf| {
            zelf.position += 1;
            false
        })
    }

    /// [`Self::next`] walking backwards, exhausted once it steps off the front.
    #[must_use = "what this hands back is released after the lock, not here"]
    pub fn rev_next<F>(&mut self, f: F) -> (PyResult<PyIterReturn>, Option<T>)
    where
        F: FnOnce(&T, usize) -> PyResult<PyIterReturn>,
    {
        self._next(f, |zelf| {
            if zelf.position == 0 {
                return true;
            }
            zelf.position -= 1;
            false
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

/// Take `step` under the lock `internal` holds, releasing whatever the step
/// hands back only after that lock is gone. `setiter_iternext()` puts its
/// `Py_DECREF(so)` past `Py_END_CRITICAL_SECTION()` for the same reason: a
/// `__del__` that iterates again would otherwise wait on a lock still held here.
pub(crate) fn locked_step<T>(
    internal: &PyMutex<PositionIterInternal<T>>,
    step: impl FnOnce(&mut PositionIterInternal<T>) -> (PyResult<PyIterReturn>, Option<T>),
) -> PyResult<PyIterReturn> {
    let mut guard = internal.lock();
    let (ret, released) = step(&mut guard);
    drop(guard);
    drop(released);
    ret
}

/// [`PositionIterInternal::next`] with the release [`locked_step`] describes.
pub fn locked_next<T, F>(
    internal: &PyMutex<PositionIterInternal<T>>,
    f: F,
) -> PyResult<PyIterReturn>
where
    F: FnOnce(&T, usize) -> PyResult<PyIterReturn>,
{
    locked_step(internal, |internal| internal.next(f))
}

/// [`locked_next`] walking backwards.
pub fn locked_rev_next<T, F>(
    internal: &PyMutex<PositionIterInternal<T>>,
    f: F,
) -> PyResult<PyIterReturn>
where
    F: FnOnce(&T, usize) -> PyResult<PyIterReturn>,
{
    locked_step(internal, |internal| internal.rev_next(f))
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
    fn __length_hint__(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        vm.with_recursion("in __length_hint__", || {
            let (obj, position) = {
                let internal = self.internal.lock();
                match &internal.status {
                    IterStatus::Active(obj) => (Some(obj.clone()), internal.position),
                    IterStatus::Exhausted => (None, 0),
                }
            };
            if let Some(obj) = obj {
                let seq = obj.sequence_unchecked();
                match seq.length_opt(vm) {
                    Some(len) => {
                        len.map(|len| PyInt::from(len.saturating_sub(position)).into_pyobject(vm))
                    }
                    None => Ok(vm.ctx.not_implemented()),
                }
            } else {
                Ok(PyInt::from(0).into_pyobject(vm))
            }
        })
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
        locked_next(&zelf.internal, |obj, pos| {
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
    #[must_use]
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

        // Re-check before comparing, but don't hold the lock while running
        // sentinel equality. User __eq__ code can re-enter this iterator.
        {
            let status = zelf.status.read();
            if !matches!(&*status, IterStatus::Active(_)) {
                return Ok(PyIterReturn::StopIteration(None));
            }
        }

        let is_sentinel = vm.identical_or_equal(&ret, &zelf.sentinel)?;

        if is_sentinel {
            let status = zelf.status.upgradable_read();
            if !matches!(&*status, IterStatus::Active(_)) {
                return Ok(PyIterReturn::StopIteration(None));
            }
            *PyRwLockUpgradableReadGuard::upgrade(status) = IterStatus::Exhausted;
            Ok(PyIterReturn::StopIteration(None))
        } else {
            Ok(PyIterReturn::Return(ret))
        }
    }
}

pub fn init(context: &'static Context) {
    PySequenceIterator::extend_class(context, context.types.iter_type);
    PyCallableIterator::extend_class(context, context.types.callable_iterator);
}
