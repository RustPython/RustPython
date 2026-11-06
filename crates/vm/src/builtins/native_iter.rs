use super::PyTupleRef;
use crate::{
    PyObject, PyObjectRef, PyResult, VirtualMachine,
    function::int_as_ssize,
    object::{Traverse, TraverseFn},
};
use rustpython_common::lock::PyMutex;

#[derive(Debug, Clone, Copy)]
pub(crate) enum NativeIterKind {
    Sequence,
    List,
    ListReverse,
    ByteArray,
}

#[derive(Debug)]
struct NativeIterState<T> {
    sequence: Option<T>,
    index: isize,
}

unsafe impl<T: Traverse> Traverse for NativeIterState<T> {
    fn traverse(&self, tracer: &mut TraverseFn<'_>) {
        self.sequence.traverse(tracer);
    }
}

// Concrete sequence reads cannot invoke Python slots. Keep the read and cursor
// update together, but allocate results and release owners after unlocking.
// Unlike generic sequence iterators, a negative native cursor can retain its
// owner: lists can restore it with __setstate__, while bytearrays cannot.
#[derive(Debug)]
pub(crate) struct NativeSequenceIter<T> {
    state: PyMutex<NativeIterState<T>>,
    kind: NativeIterKind,
}

unsafe impl<T: Traverse> Traverse for NativeSequenceIter<T> {
    fn traverse(&self, tracer: &mut TraverseFn<'_>) {
        self.state.traverse(tracer);
    }
}

impl<T> NativeSequenceIter<T> {
    pub(crate) const fn new(sequence: T, kind: NativeIterKind, index: isize) -> Self {
        Self {
            state: PyMutex::new(NativeIterState {
                sequence: Some(sequence),
                index,
            }),
            kind,
        }
    }

    // The closure may read native storage and update a cached offset, but must
    // not allocate Python objects, release Python owners, or invoke Python.
    pub(crate) fn next<R>(&self, read: impl FnOnce(&mut T, usize) -> Option<R>) -> Option<R> {
        let mut state = self.state.lock();
        if state.index < 0 {
            return None;
        }
        let index = state.index as usize;
        let sequence = state.sequence.as_mut()?;
        let item = read(sequence, index);
        let _released = if item.is_some() {
            state.index += if matches!(self.kind, NativeIterKind::ListReverse) {
                -1
            } else {
                1
            };
            None
        } else {
            state.index = -1;
            state.sequence.take()
        };
        drop(state);
        item
    }

    pub(crate) fn length_hint(&self, length: impl FnOnce(&T) -> usize) -> usize {
        let state = self.state.lock();
        if state.index < 0 {
            return 0;
        }
        let Some(sequence) = &state.sequence else {
            return 0;
        };
        let length = length(sequence);
        let index = state.index as usize;
        if matches!(self.kind, NativeIterKind::ListReverse) {
            if index < length { index + 1 } else { 0 }
        } else {
            length.saturating_sub(index)
        }
    }

    pub(crate) fn set_state(
        &self,
        value: &PyObject,
        length: impl FnOnce(&mut T) -> usize,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        // Validate even a permanently exhausted iterator, before taking its
        // lock. A rejected state must not invalidate a cached string offset.
        let index = int_as_ssize(value, vm)?;
        let mut state = self.state.lock();
        if matches!(self.kind, NativeIterKind::ByteArray) && state.index < 0 {
            return Ok(());
        }
        if let Some(sequence) = &mut state.sequence {
            let length = length(sequence) as isize;
            let minimum = if matches!(self.kind, NativeIterKind::Sequence) {
                0
            } else {
                -1
            };
            let maximum = if matches!(self.kind, NativeIterKind::ListReverse) {
                length - 1
            } else {
                length
            };
            state.index = index.clamp(minimum, maximum);
        }
        Ok(())
    }

    // Resolve the builtin before this call. Snapshot only native state while
    // locked; constructing the reduction can allocate and run finalizers.
    pub(crate) fn reduce(
        &self,
        func: PyObjectRef,
        active: impl FnOnce(T) -> PyObjectRef,
        empty: impl FnOnce(&VirtualMachine) -> PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyTupleRef
    where
        T: Clone,
    {
        let snapshot = {
            let state = self.state.lock();
            if state.index >= 0 {
                state.sequence.clone().map(|seq| (seq, state.index))
            } else {
                None
            }
        };
        match snapshot {
            Some((sequence, index)) => vm.new_tuple((func, (active(sequence),), index)),
            None => vm.new_tuple((func, (empty(vm),))),
        }
    }
}
