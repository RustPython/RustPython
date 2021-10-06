pub(crate) use _collections::make_module;

#[pymodule]
mod _collections {
    use crate::builtins::PositionIterInternal;
    use crate::common::lock::{PyMutex, PyRwLock, PyRwLockReadGuard, PyRwLockWriteGuard};
    use crate::{
        builtins::{
            IterStatus::{Active, Exhausted},
            PyInt, PyTypeRef,
        },
        function::{FuncArgs, KwArgs, OptionalArg},
        protocol::PyIterReturn,
        sequence, sliceable,
        slots::{
            Comparable, Hashable, Iterable, IteratorIterable, PyComparisonOp, SlotConstructor,
            SlotIterator, Unhashable,
        },
        vm::ReprGuard,
        PyComparisonValue, PyObjectRef, PyRef, PyResult, PyValue, TypeProtocol, VirtualMachine,
    };
    use crossbeam_utils::atomic::AtomicCell;
    use itertools::Itertools;
    use std::cmp::{max, min};
    use std::collections::VecDeque;

    #[pyattr]
    #[pyclass(name = "deque")]
    #[derive(Debug, Default, PyValue)]
    struct PyDeque {
        deque: PyRwLock<VecDeque<PyObjectRef>>,
        maxlen: Option<usize>,
        state: AtomicCell<usize>, // incremented whenever the indices move
    }

    type PyDequeRef = PyRef<PyDeque>;

    #[derive(FromArgs)]
    struct PyDequeOptions {
        #[pyarg(any, optional)]
        iterable: OptionalArg<PyObjectRef>,
        #[pyarg(any, optional)]
        maxlen: OptionalArg<PyObjectRef>,
    }

    impl PyDeque {
        fn borrow_deque(&self) -> PyRwLockReadGuard<'_, VecDeque<PyObjectRef>> {
            self.deque.read()
        }

        fn borrow_deque_mut(&self) -> PyRwLockWriteGuard<'_, VecDeque<PyObjectRef>> {
            self.deque.write()
        }
    }

    struct SimpleSeqDeque<'a>(PyRwLockReadGuard<'a, VecDeque<PyObjectRef>>);

    impl sequence::SimpleSeq for SimpleSeqDeque<'_> {
        fn len(&self) -> usize {
            self.0.len()
        }

        fn boxed_iter(&self) -> sequence::DynPyIter {
            Box::new(self.0.iter())
        }
    }

    impl<'a> From<PyRwLockReadGuard<'a, VecDeque<PyObjectRef>>> for SimpleSeqDeque<'a> {
        fn from(from: PyRwLockReadGuard<'a, VecDeque<PyObjectRef>>) -> Self {
            Self(from)
        }
    }

    #[pyimpl(flags(BASETYPE), with(Comparable, Hashable, Iterable))]
    impl PyDeque {
        #[pyslot]
        fn slot_new(cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            PyDeque::default().into_pyresult_with_type(vm, cls)
        }

        #[pymethod(magic)]
        fn init(
            zelf: PyRef<Self>,
            PyDequeOptions { iterable, maxlen }: PyDequeOptions,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            // TODO: This is _basically_ pyobject_to_opt_usize in itertools.rs
            // need to move that function elsewhere and refactor usages.
            let maxlen = if let Some(obj) = maxlen.into_option() {
                if !vm.is_none(&obj) {
                    let maxlen: isize = obj
                        .payload::<PyInt>()
                        .ok_or_else(|| vm.new_type_error("an integer is required.".to_owned()))?
                        .try_to_primitive(vm)?;

                    if maxlen.is_negative() {
                        return Err(vm.new_value_error("maxlen must be non-negative.".to_owned()));
                    }
                    Some(maxlen as usize)
                } else {
                    None
                }
            } else {
                None
            };

            // retrieve elements first to not to make too huge lock
            let elements = iterable
                .into_option()
                .map(|iter| {
                    let mut elements: Vec<PyObjectRef> = vm.extract_elements(&iter)?;
                    if let Some(maxlen) = maxlen {
                        elements.drain(..elements.len().saturating_sub(maxlen));
                    }
                    Ok(elements)
                })
                .transpose()?;

            // SAFETY: This is hacky part for read-only field
            // Because `maxlen` is only mutated from __init__. We can abuse the lock of deque to ensure this is locked enough.
            // If we make a single lock of deque not only for extend but also for setting maxlen, it will be safe.
            {
                let mut deque = zelf.borrow_deque_mut();
                // Clear any previous data present.
                deque.clear();
                unsafe {
                    // `maxlen` is better to be defined as UnsafeCell in common practice,
                    // but then more type works without any safety benefits
                    let unsafe_maxlen =
                        &zelf.maxlen as *const _ as *const std::cell::UnsafeCell<Option<usize>>;
                    *(*unsafe_maxlen).get() = maxlen;
                }
                if let Some(elements) = elements {
                    deque.extend(elements);
                }
            }

            Ok(())
        }

        #[pymethod]
        fn append(&self, obj: PyObjectRef) {
            self.state.fetch_add(1);
            let mut deque = self.borrow_deque_mut();
            if self.maxlen == Some(deque.len()) {
                deque.pop_front();
            }
            deque.push_back(obj);
        }

        #[pymethod]
        fn appendleft(&self, obj: PyObjectRef) {
            self.state.fetch_add(1);
            let mut deque = self.borrow_deque_mut();
            if self.maxlen == Some(deque.len()) {
                deque.pop_back();
            }
            deque.push_front(obj);
        }

        #[pymethod]
        fn clear(&self) {
            self.state.fetch_add(1);
            self.borrow_deque_mut().clear()
        }

        #[pymethod(magic)]
        #[pymethod]
        fn copy(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
            Self {
                deque: PyRwLock::new(zelf.borrow_deque().clone()),
                maxlen: zelf.maxlen,
                state: AtomicCell::new(zelf.state.load()),
            }
            .into_ref_with_type(vm, zelf.clone_class())
        }

        #[pymethod]
        fn count(&self, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
            let mut count = 0;
            let start_state = self.state.load();
            let deque = self.borrow_deque().clone();
            for elem in deque.iter() {
                if vm.identical_or_equal(elem, &obj)? {
                    count += 1;
                }

                if start_state != self.state.load() {
                    return Err(vm.new_runtime_error("deque mutated during iteration".to_owned()));
                }
            }
            Ok(count)
        }

        #[pymethod]
        fn extend(&self, iter: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            self.state.fetch_add(1);
            let max_len = self.maxlen;
            let mut elements: Vec<PyObjectRef> = vm.extract_elements(&iter)?;
            if let Some(max_len) = max_len {
                if max_len > elements.len() {
                    let mut deque = self.borrow_deque_mut();
                    let drain_until = deque.len().saturating_sub(max_len - elements.len());
                    deque.drain(..drain_until);
                } else {
                    self.borrow_deque_mut().clear();
                    elements.drain(..(elements.len() - max_len));
                }
            }
            self.borrow_deque_mut().extend(elements);
            Ok(())
        }

        #[pymethod]
        fn extendleft(&self, iter: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            let max_len = self.maxlen;
            let mut elements: Vec<PyObjectRef> = vm.extract_elements(&iter)?;
            elements.reverse();

            if let Some(max_len) = max_len {
                if max_len > elements.len() {
                    let mut deque = self.borrow_deque_mut();
                    let truncate_until = max_len - elements.len();
                    deque.truncate(truncate_until);
                } else {
                    self.borrow_deque_mut().clear();
                    elements.truncate(max_len);
                }
            }
            let mut created = VecDeque::from(elements);
            let mut borrowed = self.borrow_deque_mut();
            created.append(&mut borrowed);
            std::mem::swap(&mut created, &mut borrowed);
            Ok(())
        }

        fn adjust_negative_index(&self, index: isize) -> usize {
            if index.is_negative() {
                max(index + self.borrow_deque().len() as isize, 0) as usize
            } else {
                index as usize
            }
        }

        #[pymethod]
        fn index(
            &self,
            obj: PyObjectRef,
            start: OptionalArg<isize>,
            stop: OptionalArg<isize>,
            vm: &VirtualMachine,
        ) -> PyResult<usize> {
            let deque = self.borrow_deque().clone();
            let start_state = self.state.load();

            let start = self.adjust_negative_index(start.unwrap_or(0));
            let stop = min(
                self.adjust_negative_index(stop.unwrap_or_else(|| deque.len() as isize)),
                deque.len(),
            );

            for (i, elem) in deque
                .iter()
                .skip(start)
                .take(stop.saturating_sub(start))
                .enumerate()
            {
                let is_element = vm.identical_or_equal(elem, &obj)?;

                if start_state != self.state.load() {
                    return Err(vm.new_runtime_error("deque mutated during iteration".to_owned()));
                }
                if is_element {
                    return Ok(i + start);
                }
            }
            Err(vm.new_value_error(
                vm.to_repr(&obj)
                    .map(|repr| format!("{} is not in deque", repr))
                    .unwrap_or_else(|_| String::new()),
            ))
        }

        #[pymethod]
        fn insert(&self, idx: i32, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            self.state.fetch_add(1);
            let mut deque = self.borrow_deque_mut();

            if self.maxlen == Some(deque.len()) {
                return Err(vm.new_index_error("deque already at its maximum size".to_owned()));
            }

            let idx = if idx < 0 {
                if -idx as usize > deque.len() {
                    0
                } else {
                    deque.len() - ((-idx) as usize)
                }
            } else if idx as usize > deque.len() {
                deque.len()
            } else {
                idx as usize
            };

            deque.insert(idx, obj);

            Ok(())
        }

        #[pymethod]
        fn pop(&self, vm: &VirtualMachine) -> PyResult {
            self.state.fetch_add(1);
            self.borrow_deque_mut()
                .pop_back()
                .ok_or_else(|| vm.new_index_error("pop from an empty deque".to_owned()))
        }

        #[pymethod]
        fn popleft(&self, vm: &VirtualMachine) -> PyResult {
            self.state.fetch_add(1);
            self.borrow_deque_mut()
                .pop_front()
                .ok_or_else(|| vm.new_index_error("pop from an empty deque".to_owned()))
        }

        #[pymethod]
        fn remove(&self, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            let deque = self.borrow_deque().clone();
            let start_state = self.state.load();

            let mut idx = None;
            for (i, elem) in deque.iter().enumerate() {
                if vm.identical_or_equal(elem, &obj)? {
                    idx = Some(i);
                    break;
                }
            }
            let mut deque = self.borrow_deque_mut();
            if start_state != self.state.load() {
                Err(vm.new_index_error("deque mutated during remove().".to_owned()))
            } else {
                self.state.fetch_add(1);
                idx.map(|idx| deque.remove(idx).unwrap())
                    .ok_or_else(|| vm.new_value_error("deque.remove(x): x not in deque".to_owned()))
            }
        }

        #[pymethod]
        fn reverse(&self) {
            let rev: VecDeque<_> = self.borrow_deque().iter().cloned().rev().collect();
            *self.borrow_deque_mut() = rev;
        }

        #[pymethod(magic)]
        fn reversed(zelf: PyRef<Self>) -> PyResult<PyReverseDequeIterator> {
            Ok(PyReverseDequeIterator {
                state: zelf.state.load(),
                internal: PyMutex::new(PositionIterInternal::new(zelf, 0)),
            })
        }

        #[pymethod]
        fn rotate(&self, mid: OptionalArg<isize>) {
            self.state.fetch_add(1);
            let mut deque = self.borrow_deque_mut();
            if !deque.is_empty() {
                let mid = mid.unwrap_or(1) % deque.len() as isize;
                if mid.is_negative() {
                    deque.rotate_left(-mid as usize);
                } else {
                    deque.rotate_right(mid as usize);
                }
            }
        }

        #[pyproperty]
        fn maxlen(&self) -> Option<usize> {
            self.maxlen
        }

        #[pymethod(magic)]
        fn getitem(&self, idx: isize, vm: &VirtualMachine) -> PyResult {
            let deque = self.borrow_deque();
            sliceable::wrap_index(idx, deque.len())
                .and_then(|i| deque.get(i).cloned())
                .ok_or_else(|| vm.new_index_error("deque index out of range".to_owned()))
        }

        #[pymethod(magic)]
        fn setitem(&self, idx: isize, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            let mut deque = self.borrow_deque_mut();
            sliceable::wrap_index(idx, deque.len())
                .and_then(|i| deque.get_mut(i))
                .map(|x| *x = value)
                .ok_or_else(|| vm.new_index_error("deque index out of range".to_owned()))
        }

        #[pymethod(magic)]
        fn delitem(&self, idx: isize, vm: &VirtualMachine) -> PyResult<()> {
            let mut deque = self.borrow_deque_mut();
            sliceable::wrap_index(idx, deque.len())
                .and_then(|i| deque.remove(i).map(drop))
                .ok_or_else(|| vm.new_index_error("deque index out of range".to_owned()))
        }

        #[pymethod(magic)]
        fn repr(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<String> {
            let repr = if let Some(_guard) = ReprGuard::enter(vm, zelf.as_object()) {
                let deque = zelf.borrow_deque().clone();
                let elements = deque
                    .iter()
                    .map(|obj| vm.to_repr(obj))
                    .collect::<Result<Vec<_>, _>>()?;
                let maxlen = zelf
                    .maxlen
                    .map(|maxlen| format!(", maxlen={}", maxlen))
                    .unwrap_or_default();
                format!(
                    "{}([{}]{})",
                    zelf.class().name(),
                    elements.into_iter().format(", "),
                    maxlen
                )
            } else {
                "[...]".to_owned()
            };
            Ok(repr)
        }

        #[pymethod(magic)]
        fn contains(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
            let start_state = self.state.load();
            let deque = self.borrow_deque().clone();
            for element in deque.iter() {
                let is_element = vm.identical_or_equal(element, &needle)?;

                if start_state != self.state.load() {
                    return Err(vm.new_runtime_error("deque mutated during iteration".to_owned()));
                }
                if is_element {
                    return Ok(true);
                }
            }

            Ok(false)
        }

        #[pymethod(magic)]
        #[pymethod(name = "__rmul__")]
        fn mul(&self, value: isize, vm: &VirtualMachine) -> PyResult<Self> {
            let deque: SimpleSeqDeque = self.borrow_deque().into();
            let mul = sequence::seq_mul(vm, &deque, value)?;
            let skipped = self
                .maxlen
                .and_then(|maxlen| mul.len().checked_sub(maxlen))
                .unwrap_or(0);

            let deque = mul.skip(skipped).cloned().collect();
            Ok(PyDeque {
                deque: PyRwLock::new(deque),
                maxlen: self.maxlen,
                state: AtomicCell::new(0),
            })
        }

        #[pymethod(magic)]
        fn imul(zelf: PyRef<Self>, value: isize, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
            let mul_deque = zelf.mul(value, vm)?;
            std::mem::swap(
                &mut *zelf.borrow_deque_mut(),
                &mut *mul_deque.borrow_deque_mut(),
            );
            Ok(zelf)
        }

        #[pymethod(magic)]
        fn len(&self) -> usize {
            self.borrow_deque().len()
        }

        #[pymethod(magic)]
        fn bool(&self) -> bool {
            !self.borrow_deque().is_empty()
        }

        #[pymethod(magic)]
        fn add(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<Self> {
            if let Some(o) = other.payload_if_subclass::<PyDeque>(vm) {
                let mut deque = self.borrow_deque().clone();
                let elements = o.borrow_deque().clone();
                deque.extend(elements);

                let skipped = self
                    .maxlen
                    .and_then(|maxlen| deque.len().checked_sub(maxlen))
                    .unwrap_or(0);
                deque.drain(0..skipped);

                Ok(PyDeque {
                    deque: PyRwLock::new(deque),
                    maxlen: self.maxlen,
                    state: AtomicCell::new(0),
                })
            } else {
                Err(vm.new_type_error(format!(
                    "can only concatenate deque (not \"{}\") to deque",
                    other.class().name()
                )))
            }
        }

        #[pymethod(magic)]
        fn iadd(
            zelf: PyRef<Self>,
            other: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            zelf.extend(other, vm)?;
            Ok(zelf)
        }

        #[pymethod(magic)]
        fn reduce(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            let cls = zelf.clone_class().into_object();
            Ok(match zelf.maxlen {
                Some(v) => vm.ctx.new_tuple(vec![
                    cls,
                    vm.ctx.new_tuple(vec![
                        vm.ctx.empty_tuple.clone().into_object(),
                        vm.ctx.new_int(v),
                    ]),
                    vm.ctx.none(),
                    PyDequeIterator::new(zelf).into_object(vm),
                ]),
                None => vm.ctx.new_tuple(vec![
                    cls,
                    vm.ctx.empty_tuple.clone().into_object(),
                    vm.ctx.none(),
                    PyDequeIterator::new(zelf).into_object(vm),
                ]),
            })
        }
    }

    impl Comparable for PyDeque {
        fn cmp(
            zelf: &PyRef<Self>,
            other: &PyObjectRef,
            op: PyComparisonOp,
            vm: &VirtualMachine,
        ) -> PyResult<PyComparisonValue> {
            if let Some(res) = op.identical_optimization(zelf, other) {
                return Ok(res.into());
            }
            let other = class_or_notimplemented!(Self, other);
            let (lhs, rhs) = (zelf.borrow_deque(), other.borrow_deque());
            sequence::cmp(vm, Box::new(lhs.iter()), Box::new(rhs.iter()), op)
                .map(PyComparisonValue::Implemented)
        }
    }

    impl Unhashable for PyDeque {}

    impl Iterable for PyDeque {
        fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            Ok(PyDequeIterator::new(zelf).into_object(vm))
        }
    }

    #[pyattr]
    #[pyclass(name = "_deque_iterator")]
    #[derive(Debug, PyValue)]
    struct PyDequeIterator {
        state: usize,
        internal: PyMutex<PositionIterInternal<PyDequeRef>>,
    }

    #[derive(FromArgs)]
    struct DequeIterArgs {
        #[pyarg(positional)]
        deque: PyDequeRef,

        #[pyarg(positional, optional)]
        index: OptionalArg<isize>,
    }

    impl SlotConstructor for PyDequeIterator {
        type Args = (DequeIterArgs, KwArgs);

        fn py_new(
            cls: PyTypeRef,
            (DequeIterArgs { deque, index }, _kwargs): Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult {
            let iter = PyDequeIterator::new(deque);
            if let OptionalArg::Present(index) = index {
                let index = max(index, 0) as usize;
                iter.internal.lock().position = index;
            }
            iter.into_pyresult_with_type(vm, cls)
        }
    }

    #[pyimpl(with(SlotIterator, SlotConstructor))]
    impl PyDequeIterator {
        pub(crate) fn new(deque: PyDequeRef) -> Self {
            PyDequeIterator {
                state: deque.state.load(),
                internal: PyMutex::new(PositionIterInternal::new(deque, 0)),
            }
        }

        #[pymethod(magic)]
        fn length_hint(&self) -> usize {
            self.internal.lock().length_hint(|obj| obj.len())
        }

        #[pymethod(magic)]
        fn reduce(
            zelf: PyRef<Self>,
            vm: &VirtualMachine,
        ) -> (PyTypeRef, (PyDequeRef, PyObjectRef)) {
            let internal = zelf.internal.lock();
            let deque = match &internal.status {
                Active(obj) => obj.clone(),
                Exhausted => PyDeque::default().into_ref(vm),
            };
            (
                zelf.clone_class(),
                (deque, vm.ctx.new_int(internal.position)),
            )
        }
    }

    impl IteratorIterable for PyDequeIterator {}
    impl SlotIterator for PyDequeIterator {
        fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            zelf.internal.lock().next(|deque, pos| {
                if zelf.state != deque.state.load() {
                    return Err(vm.new_runtime_error("Deque mutated during iteration".to_owned()));
                }
                let deque = deque.borrow_deque();
                Ok(match deque.get(pos) {
                    Some(x) => PyIterReturn::Return(x.clone()),
                    None => PyIterReturn::StopIteration(None),
                })
            })
        }
    }

    #[pyattr]
    #[pyclass(name = "_deque_reverse_iterator")]
    #[derive(Debug, PyValue)]
    struct PyReverseDequeIterator {
        state: usize,
        // position is counting from the tail
        internal: PyMutex<PositionIterInternal<PyDequeRef>>,
    }

    impl SlotConstructor for PyReverseDequeIterator {
        type Args = (DequeIterArgs, KwArgs);

        fn py_new(
            cls: PyTypeRef,

            (DequeIterArgs { deque, index }, _kwargs): Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult {
            let iter = PyDeque::reversed(deque)?;
            if let OptionalArg::Present(index) = index {
                let index = max(index, 0) as usize;
                iter.internal.lock().position = index;
            }
            iter.into_pyresult_with_type(vm, cls)
        }
    }

    #[pyimpl(with(SlotIterator, SlotConstructor))]
    impl PyReverseDequeIterator {
        #[pymethod(magic)]
        fn length_hint(&self) -> usize {
            self.internal.lock().length_hint(|obj| obj.len())
        }

        #[pymethod(magic)]
        fn reduce(
            zelf: PyRef<Self>,
            vm: &VirtualMachine,
        ) -> PyResult<(PyTypeRef, (PyDequeRef, PyObjectRef))> {
            let internal = zelf.internal.lock();
            let deque = match &internal.status {
                Active(obj) => obj.clone(),
                Exhausted => PyDeque::default().into_ref(vm),
            };
            Ok((
                zelf.clone_class(),
                (deque, vm.ctx.new_int(internal.position)),
            ))
        }
    }

    impl IteratorIterable for PyReverseDequeIterator {}
    impl SlotIterator for PyReverseDequeIterator {
        fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            zelf.internal.lock().next(|deque, pos| {
                if deque.state.load() != zelf.state {
                    return Err(vm.new_runtime_error("Deque mutated during iteration".to_owned()));
                }
                let deque = deque.borrow_deque();
                Ok(
                    match deque
                        .len()
                        .checked_sub(pos + 1)
                        .and_then(|pos| deque.get(pos))
                    {
                        Some(x) => PyIterReturn::Return(x.clone()),
                        None => PyIterReturn::StopIteration(None),
                    },
                )
            })
        }
    }
}
