/*
 * iterator types
 */

use super::{PyInt, PyTupleRef, PyType};
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyResult, VirtualMachine,
    class::PyClassImpl,
    function::{ArgCallable, int_as_ssize},
    object::{Traverse, TraverseFn},
    protocol::PyIterReturn,
    types::{IterNext, Iterable, PyComparisonOp, SelfIter},
};
use rustpython_common::lock::{PyMutex, PyRwLock};

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

    pub fn set_state<F>(&mut self, state: &PyObject, f: F, vm: &VirtualMachine) -> PyResult<()>
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
        let done = match &ret {
            Ok(PyIterReturn::Return(_)) => op(self),
            Ok(PyIterReturn::StopIteration(_)) => true,
            // An error belongs to the element, not to the walk, so the next
            // call reaches for the same one again. `iter_iternext()` lets go of
            // its sequence for `IndexError` and `StopIteration` alone, and
            // `PyIterReturn::from_getitem_result` has already turned the first
            // of those into the second.
            Err(_) => false,
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
    let (ret, _released) = step(&mut guard);
    drop(guard);
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

pub fn builtins_iter(vm: &VirtualMachine) -> PyResult {
    vm.eval_get_builtin(vm.ctx.intern_str("iter"))
}

pub fn builtins_reversed(vm: &VirtualMachine) -> PyResult {
    vm.eval_get_builtin(vm.ctx.intern_str("reversed"))
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum SequenceIterDirection {
    Forward,
    Reverse,
}

#[derive(Debug, Traverse)]
struct SequenceIterState {
    sequence: Option<PyObjectRef>,
    #[pytraverse(skip)]
    index: isize,
}

/// State for generic iterators whose sequence slots can execute Python code.
/// Unlike the concrete container iterators, these must release their state
/// lock before invoking a slot, allocating a Python object, or dropping an owner.
#[derive(Debug, Traverse)]
pub(crate) struct SequenceIter {
    state: PyMutex<SequenceIterState>,
    #[pytraverse(skip)]
    direction: SequenceIterDirection,
}

impl SequenceIter {
    pub(crate) const fn new(
        sequence: PyObjectRef,
        index: isize,
        direction: SequenceIterDirection,
    ) -> Self {
        Self {
            state: PyMutex::new(SequenceIterState {
                sequence: Some(sequence),
                index,
            }),
            direction,
        }
    }

    fn snapshot(&self) -> (Option<PyObjectRef>, isize) {
        let state = self.state.lock();
        (state.sequence.clone(), state.index)
    }

    fn exhaust(&self) {
        let _released = {
            let mut state = self.state.lock();
            if matches!(self.direction, SequenceIterDirection::Reverse) {
                state.index = -1;
            }
            state.sequence.take()
        };
    }

    fn get_item(sequence: &PyObject, mut index: isize, vm: &VirtualMachine) -> PyResult {
        let seq = sequence.sequence_unchecked();
        // A reentrant state change can wrap the forward index below zero.
        // PySequence_GetItem adjusts it once using the sequence length slot.
        // Keep this separate from the concrete containers' index handling.
        if index < 0
            && seq.slots().has_item()
            && let Some(length) = seq.length_opt(vm)
        {
            index = index.wrapping_add(length? as isize);
        }
        seq.get_item(index, vm)
    }

    pub(crate) fn next(&self, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        let (sequence, index) = self.snapshot();
        let Some(sequence) = sequence else {
            return Ok(PyIterReturn::StopIteration(None));
        };
        match self.direction {
            SequenceIterDirection::Forward if index == isize::MAX => {
                drop(sequence);
                return Err(vm.new_overflow_error("iter index too large"));
            }
            SequenceIterDirection::Reverse if index < 0 => {
                drop(sequence);
                self.exhaust();
                return Ok(PyIterReturn::StopIteration(None));
            }
            _ => {}
        }

        let result = Self::get_item(&sequence, index, vm);
        // A reentrant call can have removed the iterator's sequence owner.
        // Release our temporary owner before publishing the outer result.
        drop(sequence);
        match result {
            Ok(item) => {
                let mut state = self.state.lock();
                state.index = match self.direction {
                    // Forward iteration advances the current index, including
                    // changes made by __getitem__ or a nested next().
                    // CPython's default no-strict-overflow build wraps if
                    // that callback restored PY_SSIZE_T_MAX.
                    SequenceIterDirection::Forward => state.index.wrapping_add(1),
                    // Reverse iteration instead advances the captured index.
                    // Keep the sequence at -1 until the next next() call.
                    SequenceIterDirection::Reverse => index - 1,
                };
                Ok(PyIterReturn::Return(item))
            }
            Err(error)
                if error.fast_isinstance(vm.ctx.exceptions.index_error)
                    || error.fast_isinstance(vm.ctx.exceptions.stop_iteration) =>
            {
                // Clear the whole exception, including StopIteration.value,
                // before publishing exhaustion. Its finalizer may reenter.
                drop(error);
                self.exhaust();
                Ok(PyIterReturn::StopIteration(None))
            }
            Err(error) => {
                if matches!(self.direction, SequenceIterDirection::Reverse) {
                    self.exhaust();
                }
                Err(error)
            }
        }
    }

    pub(crate) fn length_hint(&self, vm: &VirtualMachine) -> PyResult<Option<usize>> {
        let (sequence, index) = self.snapshot();
        let Some(sequence) = sequence else {
            return Ok(Some(0));
        };
        if matches!(self.direction, SequenceIterDirection::Reverse) && index < 0 {
            return Ok(Some(0));
        }
        let seq = sequence.sequence_unchecked();
        if matches!(self.direction, SequenceIterDirection::Forward)
            && seq.slots().length.load().is_none()
            && sequence.mapping_unchecked().slots().length.load().is_none()
        {
            return Ok(None);
        }
        let length = seq.length(vm)?;
        drop(sequence);
        let remaining = match self.direction {
            SequenceIterDirection::Forward => {
                // iter_len reads the index after the length callback.
                let index = self.state.lock().index;
                (length as isize).wrapping_sub(index).max(0) as usize
            }
            SequenceIterDirection::Reverse => {
                // reversed_len uses the index captured before the callback.
                let remaining = index as usize + 1;
                if length < remaining { 0 } else { remaining }
            }
        };
        Ok(Some(remaining))
    }

    pub(crate) fn set_state(&self, state: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
        // PyLong_AsSsize_t accepts int subclasses, not __index__. Convert even
        // when exhausted, and retain the original argument through callbacks.
        let index = int_as_ssize(state, vm)?;
        match self.direction {
            SequenceIterDirection::Forward => {
                let mut state = self.state.lock();
                if state.sequence.is_some() {
                    state.index = index.max(0);
                }
            }
            SequenceIterDirection::Reverse => {
                let (sequence, current) = self.snapshot();
                if current < 0 {
                    return Ok(());
                }
                let Some(sequence) = sequence else {
                    return Ok(());
                };
                let length = sequence.sequence_unchecked().length(vm)?;
                drop(sequence);
                self.state.lock().index = index.clamp(-1, length as isize - 1);
            }
        }
        Ok(())
    }

    /// Resolve the reconstruction function before taking this snapshot. Tuple
    /// allocation can also run GC and finalizers, so it follows the lock scope.
    pub(crate) fn reduce(&self, func: PyObjectRef, vm: &VirtualMachine) -> PyTupleRef {
        let (sequence, index) = self.snapshot();
        if let Some(sequence) = sequence
            && (matches!(self.direction, SequenceIterDirection::Forward) || index >= 0)
        {
            vm.new_tuple((func, (sequence,), index))
        } else {
            vm.new_tuple((func, (vm.ctx.empty_tuple.clone(),)))
        }
    }
}

#[pyclass(module = false, name = "iterator", traverse)]
#[derive(Debug)]
pub struct PySequenceIterator {
    internal: SequenceIter,
}

impl PyPayload for PySequenceIterator {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.iter_type
    }
}

impl PySequenceIterator {
    pub fn new(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<Self> {
        let _seq = obj.try_sequence(vm)?;
        Ok(Self {
            internal: SequenceIter::new(obj, 0, SequenceIterDirection::Forward),
        })
    }
}

#[pyclass(with(IterNext, Iterable))]
impl Py<PySequenceIterator> {
    #[pymethod]
    fn __length_hint__(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        vm.with_recursion(" in __length_hint__", || {
            Ok(match self.internal.length_hint(vm)? {
                Some(length) => PyInt::from(length).into_pyobject(vm),
                None => vm.ctx.not_implemented(),
            })
        })
    }

    #[pymethod]
    fn __reduce__(&self, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
        let func = builtins_iter(vm)?;
        Ok(self.internal.reduce(func, vm))
    }

    #[pymethod]
    fn __setstate__(&self, state: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.internal.set_state(&state, vm)
    }
}

impl SelfIter for PySequenceIterator {}
impl IterNext for PySequenceIterator {
    fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        zelf.internal.next(vm)
    }
}

#[pyclass(module = false, name = "callable_iterator", traverse)]
#[derive(Debug)]
pub struct PyCallableIterator {
    status: PyRwLock<IterStatus<(ArgCallable, PyObjectRef)>>,
}

impl PyPayload for PyCallableIterator {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.callable_iterator
    }
}

impl PyCallableIterator {
    #[must_use]
    pub const fn new(callable: ArgCallable, sentinel: PyObjectRef) -> Self {
        Self {
            status: PyRwLock::new(IterStatus::Active((callable, sentinel))),
        }
    }

    fn exhaust(&self) {
        let _released = {
            let mut status = self.status.write();
            core::mem::replace(&mut *status, IterStatus::Exhausted)
        };
        // Dropping the callable or sentinel can invoke a finalizer that calls
        // next() again, so both must be released after the state lock.
    }
}

#[pyclass(with(IterNext, Iterable))]
impl Py<PyCallableIterator> {
    #[pymethod]
    fn __reduce__(&self, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
        let func = builtins_iter(vm)?;
        let active = {
            let status = self.status.read();
            match &*status {
                IterStatus::Active((callable, sentinel)) => {
                    let callable: PyObjectRef = callable.clone().into();
                    Some((callable, sentinel.clone()))
                }
                IterStatus::Exhausted => None,
            }
        };
        // Tuple allocation may collect objects whose finalizers re-enter next().
        if let Some(args) = active {
            Ok(vm.new_tuple((func, args)))
        } else {
            Ok(vm.new_tuple((func, (vm.ctx.empty_tuple.clone(),))))
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
                IterStatus::Active((callable, _)) => callable.clone(),
                IterStatus::Exhausted => return Ok(PyIterReturn::StopIteration(None)),
            }
        };

        let ret = callable.invoke((), vm);
        drop(callable);
        let ret = match PyIterReturn::from_pyresult(ret, vm)? {
            PyIterReturn::Return(ret) => ret,
            PyIterReturn::StopIteration(value) => {
                drop(value);
                zelf.exhaust();
                return Ok(PyIterReturn::StopIteration(None));
            }
        };

        // Re-check after calling, and release the lock before equality: either
        // callback can re-enter and exhaust this iterator.
        let sentinel = {
            let status = zelf.status.read();
            match &*status {
                IterStatus::Active((_, sentinel)) => sentinel.clone(),
                IterStatus::Exhausted => return Ok(PyIterReturn::StopIteration(None)),
            }
        };
        let is_sentinel = if sentinel.is(&ret) {
            drop(sentinel);
            true
        } else {
            // Consume the snapshot before truth conversion: __bool__ can
            // exhaust us and observe the sentinel's finalizer immediately.
            let comparison = sentinel.rich_compare(ret.clone(), PyComparisonOp::Eq, vm)?;
            comparison.try_to_bool(vm)?
        };
        if is_sentinel {
            zelf.exhaust();
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
