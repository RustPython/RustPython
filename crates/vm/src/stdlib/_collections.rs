// cspell:ignore odict

pub(crate) use _collections::module_def;

#[pymodule]
mod _collections {
    use crate::{
        AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
        atomic_func,
        builtins::{
            IterStatus::{Active, Exhausted},
            PositionIterInternal, PyDict, PyGenericAlias, PyInt, PyStr, PyStrRef, PyType,
            PyTypeRef, locked_step,
        },
        common::{
            ascii,
            lock::{PyMutex, PyRwLock, PyRwLockReadGuard, PyRwLockWriteGuard},
        },
        convert::ToPyObject,
        dict_inner,
        function::{ArgIterable, FuncArgs, KwArgs, OptionalArg, PyComparisonValue},
        iter::PyExactSizeIterator,
        object::{Traverse, TraverseFn},
        protocol::{PyIterReturn, PyMappingMethods, PyNumberMethods, PySequenceMethods},
        recursion::ReprGuard,
        sequence::{MutObjectSequenceOp, OptionalRangeArgs},
        sliceable::SequenceIndexOp,
        types::{
            AsMapping, AsNumber, AsSequence, Callable, Comparable, Constructor, DefaultConstructor,
            Initializer, IterNext, Iterable, PyComparisonOp, Representable, SelfIter,
        },
        utils::collection_repr,
        vm::MAX_MEMORY_SIZE,
    };
    use alloc::collections::VecDeque;
    use core::{cmp::max, mem::size_of};
    use crossbeam_utils::atomic::AtomicCell;

    #[pyattr]
    #[pyclass(
        module = "collections",
        name = "deque",
        unhashable = true,
        traverse = "manual"
    )]
    #[derive(Debug, Default, PyPayload)]
    struct PyDeque {
        deque: PyRwLock<VecDeque<PyObjectRef>>,
        maxlen: Option<usize>,
        state: AtomicCell<usize>, // incremented whenever the indices move
    }

    // SAFETY: Traverse visits each owned Python reference at most once.
    unsafe impl Traverse for PyDeque {
        fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
            if let Some(deque) = self.deque.try_read_recursive() {
                for obj in deque.iter() {
                    obj.traverse(tracer_fn);
                }
            }
        }

        fn clear(&mut self, out: &mut Vec<PyObjectRef>) {
            out.extend(self.deque.get_mut().drain(..));
        }
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

        fn is_over_maxlen(&self, deque: &VecDeque<PyObjectRef>) -> bool {
            self.maxlen.is_some_and(|maxlen| deque.len() > maxlen)
        }
    }

    #[pyclass(
        flags(BASETYPE, HAS_WEAKREF),
        with(
            Constructor,
            Initializer,
            AsNumber,
            AsSequence,
            Comparable,
            Iterable,
            Representable
        )
    )]
    impl PyDeque {
        #[pymethod]
        fn append(&self, obj: PyObjectRef) {
            self.state.fetch_add(1);
            let mut deque = self.borrow_deque_mut();
            deque.push_back(obj);
            // Trim after pushing, so that a `maxlen` of zero drops what just
            // arrived instead of popping from an empty deque and keeping it.
            if self.is_over_maxlen(&deque) {
                deque.pop_front();
            }
        }

        #[pymethod]
        fn appendleft(&self, obj: PyObjectRef) {
            self.state.fetch_add(1);
            let mut deque = self.borrow_deque_mut();
            deque.push_front(obj);
            if self.is_over_maxlen(&deque) {
                deque.pop_back();
            }
        }

        #[pymethod]
        fn clear(&self) {
            self.state.fetch_add(1);
            self.borrow_deque_mut().clear()
        }

        #[pymethod(name = "__copy__")]
        #[pymethod]
        fn copy(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
            Self {
                deque: PyRwLock::new(zelf.borrow_deque().clone()),
                maxlen: zelf.maxlen,
                state: AtomicCell::new(zelf.state.load()),
            }
            .into_ref_with_type(vm, zelf.class().to_owned())
        }

        #[pymethod]
        fn count(&self, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
            let start_state = self.state.load();
            let count = self.mut_count(vm, &obj)?;

            if start_state != self.state.load() {
                return Err(vm.new_runtime_error("deque mutated during iteration"));
            }
            Ok(count)
        }

        #[pymethod]
        fn extend(&self, iter: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            self._extend(&iter, vm)
        }

        fn _extend(&self, iter: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
            self.state.fetch_add(1);
            let max_len = self.maxlen;
            let mut elements: Vec<PyObjectRef> = iter.try_to_value(vm)?;
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
            let mut elements: Vec<PyObjectRef> = iter.try_to_value(vm)?;
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
            core::mem::swap(&mut created, &mut borrowed);
            Ok(())
        }

        #[pymethod]
        fn index(
            &self,
            needle: PyObjectRef,
            range: OptionalRangeArgs,
            vm: &VirtualMachine,
        ) -> PyResult<usize> {
            let start_state = self.state.load();

            let (start, stop) = range.saturate(self.__len__(), vm)?;
            let index = self.mut_index_range(vm, &needle, start..stop)?;
            if start_state != self.state.load() {
                Err(vm.new_runtime_error("deque mutated during iteration"))
            } else if let Some(index) = index.into() {
                Ok(index)
            } else {
                Err(vm.new_value_error(
                    needle
                        .repr(vm)
                        .map_or_else(|_| String::new(), |repr| format!("{repr} is not in deque")),
                ))
            }
        }

        #[pymethod]
        fn insert(&self, idx: i32, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            self.state.fetch_add(1);
            let mut deque = self.borrow_deque_mut();

            if self.maxlen == Some(deque.len()) {
                return Err(vm.new_index_error("deque already at its maximum size"));
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
                .ok_or_else(|| vm.new_index_error("pop from an empty deque"))
        }

        #[pymethod]
        fn popleft(&self, vm: &VirtualMachine) -> PyResult {
            self.state.fetch_add(1);
            self.borrow_deque_mut()
                .pop_front()
                .ok_or_else(|| vm.new_index_error("pop from an empty deque"))
        }

        #[pymethod]
        fn remove(&self, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            let start_state = self.state.load();
            let index = self.mut_index(vm, &obj)?;

            if start_state != self.state.load() {
                Err(vm.new_index_error("deque mutated during remove()."))
            } else if let Some(index) = index.into() {
                let mut deque = self.borrow_deque_mut();
                self.state.fetch_add(1);
                Ok(deque.remove(index).unwrap())
            } else {
                Err(vm.new_value_error("deque.remove(x): x not in deque"))
            }
        }

        #[pymethod]
        fn reverse(&self) {
            let rev: VecDeque<_> = self.borrow_deque().iter().cloned().rev().collect();
            *self.borrow_deque_mut() = rev;
        }

        #[pymethod]
        fn __reversed__(zelf: PyRef<Self>) -> PyReverseDequeIterator {
            PyReverseDequeIterator {
                state: zelf.state.load(),
                counter: AtomicCell::new(zelf.__len__()),
                internal: PyMutex::new(PositionIterInternal::new(zelf, 0)),
            }
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

        #[pygetset]
        const fn maxlen(&self) -> Option<usize> {
            self.maxlen
        }

        fn __getitem__(&self, idx: isize, vm: &VirtualMachine) -> PyResult {
            let deque = self.borrow_deque();
            idx.wrapped_at(deque.len())
                .and_then(|i| deque.get(i).cloned())
                .ok_or_else(|| vm.new_index_error("deque index out of range"))
        }

        fn __setitem__(&self, idx: isize, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            let mut deque = self.borrow_deque_mut();
            idx.wrapped_at(deque.len())
                .and_then(|i| deque.get_mut(i))
                .map(|x| *x = value)
                .ok_or_else(|| vm.new_index_error("deque index out of range"))
        }

        fn __delitem__(&self, idx: isize, vm: &VirtualMachine) -> PyResult<()> {
            let mut deque = self.borrow_deque_mut();
            idx.wrapped_at(deque.len())
                .and_then(|i| deque.remove(i).map(drop))
                .ok_or_else(|| vm.new_index_error("deque index out of range"))
        }

        fn __contains__(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
            self._contains(&needle, vm)
        }

        fn _contains(&self, needle: &PyObject, vm: &VirtualMachine) -> PyResult<bool> {
            let start_state = self.state.load();
            let ret = self.mut_contains(vm, needle)?;
            if start_state != self.state.load() {
                Err(vm.new_runtime_error("deque mutated during iteration"))
            } else {
                Ok(ret)
            }
        }

        fn _mul(&self, n: isize, vm: &VirtualMachine) -> PyResult<VecDeque<PyObjectRef>> {
            let deque = self.borrow_deque();
            let n = vm.check_repeat_or_overflow_error(deque.len(), n)?;
            let mul_len = n * deque.len();
            let result_len = self.maxlen.map_or(mul_len, |maxlen| mul_len.min(maxlen));
            if n > 1 && result_len.saturating_mul(size_of::<PyObjectRef>()) >= MAX_MEMORY_SIZE {
                return Err(vm.new_memory_error(""));
            }
            let iter = deque.iter().cycle().take(mul_len);
            let skipped = self
                .maxlen
                .and_then(|maxlen| mul_len.checked_sub(maxlen))
                .unwrap_or(0);

            let deque = iter.skip(skipped).cloned().collect();
            Ok(deque)
        }

        fn __mul__(&self, n: isize, vm: &VirtualMachine) -> PyResult<Self> {
            let deque = self._mul(n, vm)?;
            Ok(Self {
                deque: PyRwLock::new(deque),
                maxlen: self.maxlen,
                state: AtomicCell::new(0),
            })
        }

        fn __imul__(zelf: PyRef<Self>, n: isize, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
            let mul_deque = zelf._mul(n, vm)?;
            *zelf.borrow_deque_mut() = mul_deque;
            Ok(zelf)
        }

        fn __len__(&self) -> usize {
            self.borrow_deque().len()
        }

        fn concat(&self, other: &PyObject, vm: &VirtualMachine) -> PyResult<Self> {
            if let Some(o) = other.downcast_ref::<Self>() {
                let mut deque = self.borrow_deque().clone();
                let elements = o.borrow_deque().clone();
                deque.extend(elements);

                let skipped = self
                    .maxlen
                    .and_then(|maxlen| deque.len().checked_sub(maxlen))
                    .unwrap_or(0);
                deque.drain(..skipped);

                Ok(Self {
                    deque: PyRwLock::new(deque),
                    maxlen: self.maxlen,
                    state: AtomicCell::new(0),
                })
            } else {
                Err(vm.new_type_error(format!(
                    r#"can only concatenate deque (not "{}") to deque"#,
                    other.class().name()
                )))
            }
        }

        fn __iadd__(
            zelf: PyRef<Self>,
            other: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            zelf.extend(other, vm)?;
            Ok(zelf)
        }

        #[pymethod]
        fn __reduce__(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            let cls = zelf.class().to_owned();
            let value = match zelf.maxlen {
                Some(v) => vm.new_pyobj((vm.ctx.empty_tuple.clone(), v)),
                None => vm.ctx.empty_tuple.clone().into(),
            };
            // Use __getstate__ to capture both __dict__ and __slots__ values so
            // subclass attributes survive a pickle round-trip (matches CPython's
            // deque___reduce___impl, which calls _PyObject_GetState).
            let state = vm.call_method(zelf.as_object(), "__getstate__", ())?;
            Ok(vm.new_pyobj((cls, value, state, PyDequeIterator::new(zelf))))
        }

        #[pyclassmethod]
        fn __class_getitem__(
            cls: PyTypeRef,
            args: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyGenericAlias> {
            PyGenericAlias::from_args(cls, args, vm)
        }
    }

    impl MutObjectSequenceOp for PyDeque {
        type Inner = VecDeque<PyObjectRef>;

        fn do_get(index: usize, inner: &Self::Inner) -> Option<&PyObject> {
            inner.get(index).map(|r| r.as_ref())
        }

        fn do_lock(&self) -> impl core::ops::Deref<Target = Self::Inner> {
            self.borrow_deque()
        }
    }

    impl DefaultConstructor for PyDeque {}

    impl Initializer for PyDeque {
        type Args = PyDequeOptions;

        fn init(
            zelf: PyRef<Self>,
            PyDequeOptions { iterable, maxlen }: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            // TODO: This is _basically_ pyobject_to_opt_usize in itertools.rs
            // need to move that function elsewhere and refactor usages.
            let maxlen = if let Some(obj) = maxlen.into_option() {
                if !vm.is_none(&obj) {
                    let maxlen: isize = obj
                        .downcast_ref::<PyInt>()
                        .ok_or_else(|| vm.new_type_error("an integer is required."))?
                        .try_to_primitive(vm)?;

                    if maxlen.is_negative() {
                        return Err(vm.new_value_error("maxlen must be non-negative."));
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
                    let mut elements: Vec<PyObjectRef> = iter.try_to_value(vm)?;
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
                        &zelf.maxlen as *const _ as *const core::cell::UnsafeCell<Option<usize>>;
                    *(*unsafe_maxlen).get() = maxlen;
                }
                if let Some(elements) = elements {
                    deque.extend(elements);
                }
            }

            Ok(())
        }
    }

    impl AsNumber for PyDeque {
        fn as_number() -> &'static PyNumberMethods {
            static AS_NUMBER: PyNumberMethods = PyNumberMethods {
                boolean: Some(|number, _vm| {
                    let zelf = number.obj.downcast_ref::<PyDeque>().unwrap();
                    Ok(!zelf.borrow_deque().is_empty())
                }),
                ..PyNumberMethods::NOT_IMPLEMENTED
            };
            &AS_NUMBER
        }
    }

    impl AsSequence for PyDeque {
        fn as_sequence() -> &'static PySequenceMethods {
            static AS_SEQUENCE: PySequenceMethods = PySequenceMethods {
                length: atomic_func!(|seq, _vm| Ok(PyDeque::sequence_downcast(seq).__len__())),
                concat: atomic_func!(|seq, other, vm| {
                    PyDeque::sequence_downcast(seq)
                        .concat(other, vm)
                        .map(|x| x.into_ref(&vm.ctx).into())
                }),

                repeat: atomic_func!(|seq, n, vm| {
                    PyDeque::sequence_downcast(seq)
                        .__mul__(n, vm)
                        .map(|x| x.into_ref(&vm.ctx).into())
                }),

                item: atomic_func!(|seq, i, vm| PyDeque::sequence_downcast(seq).__getitem__(i, vm)),
                ass_item: atomic_func!(|seq, i, value, vm| {
                    let zelf = PyDeque::sequence_downcast(seq);
                    if let Some(value) = value {
                        zelf.__setitem__(i, value, vm)
                    } else {
                        zelf.__delitem__(i, vm)
                    }
                }),

                contains: atomic_func!(
                    |seq, needle, vm| PyDeque::sequence_downcast(seq)._contains(needle, vm)
                ),

                inplace_concat: atomic_func!(|seq, other, vm| {
                    let zelf = PyDeque::sequence_downcast(seq);
                    zelf._extend(other, vm)?;
                    Ok(zelf.to_owned().into())
                }),

                inplace_repeat: atomic_func!(|seq, n, vm| {
                    let zelf = PyDeque::sequence_downcast(seq);
                    PyDeque::__imul__(zelf.to_owned(), n, vm).map(|x| x.into())
                }),
            };

            &AS_SEQUENCE
        }
    }

    impl Comparable for PyDeque {
        fn cmp(
            zelf: &Py<Self>,
            other: &PyObject,
            op: PyComparisonOp,
            vm: &VirtualMachine,
        ) -> PyResult<PyComparisonValue> {
            if let Some(res) = op.identical_optimization(zelf, other) {
                return Ok(res.into());
            }

            let other = class_or_notimplemented!(Self, other);
            let lhs = zelf.borrow_deque();
            let rhs = other.borrow_deque();
            lhs.iter()
                .richcompare(rhs.iter(), op, vm)
                .map(PyComparisonValue::Implemented)
        }
    }

    impl Iterable for PyDeque {
        fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            Ok(PyDequeIterator::new(zelf).into_pyobject(vm))
        }
    }

    impl Representable for PyDeque {
        #[inline]
        fn repr(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyRef<PyStr>> {
            let deque = zelf.borrow_deque().clone();
            let class = zelf.class();
            let class_name = class.name();
            let closing_part = zelf
                .maxlen
                .map_or_else(|| "]".to_owned(), |maxlen| format!("], maxlen={maxlen}"));
            let empty = format!("{class_name}([{closing_part})");

            if zelf.__len__() == 0 {
                return Ok(vm.ctx.new_str(empty));
            }

            if let Some(_guard) = ReprGuard::enter(vm, zelf.as_object()) {
                Ok(vm.ctx.new_str(collection_repr(
                    Some(&class_name),
                    "[",
                    &closing_part,
                    &empty,
                    deque.iter(),
                    vm,
                )?))
            } else {
                Ok(vm.ctx.intern_str("[...]").to_owned())
            }
        }

        fn repr_str(_zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            unreachable!("repr() is overridden directly")
        }
    }

    #[pyattr]
    #[pyclass(name = "_deque_iterator")]
    #[derive(Debug, PyPayload)]
    struct PyDequeIterator {
        state: usize,
        /// How many elements are left to walk, `dequeiterobject.counter`. Kept
        /// beside the deque rather than read back from it, because a mutated
        /// deque is walked no further and what is left of it then reads as
        /// nothing.
        counter: AtomicCell<usize>,
        internal: PyMutex<PositionIterInternal<PyDequeRef>>,
    }

    #[derive(FromArgs)]
    struct DequeIterArgs {
        #[pyarg(positional)]
        deque: PyDequeRef,

        #[pyarg(positional, optional)]
        index: OptionalArg<isize>,
    }

    impl Constructor for PyDequeIterator {
        type Args = (DequeIterArgs, KwArgs);

        fn py_new(
            _cls: &Py<PyType>,
            (DequeIterArgs { deque, index }, _kwargs): Self::Args,
            _vm: &VirtualMachine,
        ) -> PyResult<Self> {
            let iter = Self::new(deque);
            if let OptionalArg::Present(index) = index {
                let index = max(index, 0) as usize;
                iter.internal.lock().position = index;
                iter.counter
                    .store(iter.counter.load().saturating_sub(index));
            }
            Ok(iter)
        }
    }

    #[pyclass(with(IterNext, Iterable, Constructor))]
    impl PyDequeIterator {
        pub(crate) fn new(deque: PyDequeRef) -> Self {
            Self {
                state: deque.state.load(),
                counter: AtomicCell::new(deque.__len__()),
                internal: PyMutex::new(PositionIterInternal::new(deque, 0)),
            }
        }

        #[pymethod]
        fn __length_hint__(&self) -> usize {
            self.counter.load()
        }

        #[pymethod]
        fn __reduce__(
            zelf: PyRef<Self>,
            vm: &VirtualMachine,
        ) -> (PyTypeRef, (PyDequeRef, PyObjectRef)) {
            let internal = zelf.internal.lock();
            let deque = match &internal.status {
                Active(obj) => obj.clone(),
                Exhausted => PyDeque::default().into_ref(&vm.ctx),
            };
            (
                zelf.class().to_owned(),
                (deque, vm.ctx.new_int(internal.position).into()),
            )
        }
    }

    impl SelfIter for PyDequeIterator {}

    /// Whether the deque moved under an iterator that captured `state`. What is
    /// left to walk is emptied before the error goes out, the way
    /// `deque_iternext()` zeroes its counter before it raises.
    fn deque_moved(
        internal: &PositionIterInternal<PyDequeRef>,
        state: usize,
        counter: &AtomicCell<usize>,
    ) -> bool {
        let Active(deque) = &internal.status else {
            return false;
        };
        if state == deque.state.load() {
            return false;
        }
        counter.store(0);
        true
    }

    /// Hand back the element at the position the iterator keeps, `at` reaching
    /// for it. Both deque iterators end here; they differ in whether they look
    /// at the deque or at the count first.
    fn deque_take(
        internal: &mut PositionIterInternal<PyDequeRef>,
        counter: &AtomicCell<usize>,
        at: impl FnOnce(&VecDeque<PyObjectRef>, usize) -> Option<PyObjectRef>,
    ) -> (PyResult<PyIterReturn>, Option<PyDequeRef>) {
        let item = match &internal.status {
            Active(deque) if counter.load() != 0 => at(&deque.borrow_deque(), internal.position),
            _ => None,
        };
        let Some(item) = item else {
            counter.store(0);
            return (Ok(PyIterReturn::StopIteration(None)), internal.exhaust());
        };
        internal.position += 1;
        counter.store(counter.load() - 1);
        (Ok(PyIterReturn::Return(item)), None)
    }

    fn deque_mutated(vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        Err(vm.new_runtime_error("deque mutated during iteration"))
    }

    impl IterNext for PyDequeIterator {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            locked_step(&zelf.internal, |internal| {
                // The deque before the count, as in `deque_iternext()`, so an
                // iterator still holding a deque that moved raises again on
                // every call rather than running out after the first.
                if deque_moved(internal, zelf.state, &zelf.counter) {
                    return (deque_mutated(vm), None);
                }
                deque_take(internal, &zelf.counter, |deque, pos| {
                    deque.get(pos).cloned()
                })
            })
        }
    }

    #[pyattr]
    #[pyclass(name = "_deque_reverse_iterator")]
    #[derive(Debug, PyPayload)]
    struct PyReverseDequeIterator {
        state: usize,
        /// As in [`PyDequeIterator`].
        counter: AtomicCell<usize>,
        // position is counting from the tail
        internal: PyMutex<PositionIterInternal<PyDequeRef>>,
    }

    impl Constructor for PyReverseDequeIterator {
        type Args = (DequeIterArgs, KwArgs);

        fn py_new(
            _cls: &Py<PyType>,
            (DequeIterArgs { deque, index }, _kwargs): Self::Args,
            _vm: &VirtualMachine,
        ) -> PyResult<Self> {
            let iter = PyDeque::__reversed__(deque);
            if let OptionalArg::Present(index) = index {
                let index = max(index, 0) as usize;
                iter.internal.lock().position = index;
                iter.counter
                    .store(iter.counter.load().saturating_sub(index));
            }
            Ok(iter)
        }
    }

    #[pyclass(with(IterNext, Iterable, Constructor))]
    impl PyReverseDequeIterator {
        #[pymethod]
        fn __length_hint__(&self) -> usize {
            self.counter.load()
        }

        #[pymethod]
        fn __reduce__(
            zelf: PyRef<Self>,
            vm: &VirtualMachine,
        ) -> (PyTypeRef, (PyDequeRef, PyObjectRef)) {
            let internal = zelf.internal.lock();
            let deque = match &internal.status {
                Active(obj) => obj.clone(),
                Exhausted => PyDeque::default().into_ref(&vm.ctx),
            };
            (
                zelf.class().to_owned(),
                (deque, vm.ctx.new_int(internal.position).into()),
            )
        }
    }

    impl SelfIter for PyReverseDequeIterator {}

    impl IterNext for PyReverseDequeIterator {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            locked_step(&zelf.internal, |internal| {
                // The count before the deque, as in `dequereviter_next()`, so
                // an iterator that has raised once runs out instead.
                if zelf.counter.load() != 0 && deque_moved(internal, zelf.state, &zelf.counter) {
                    return (deque_mutated(vm), None);
                }
                deque_take(internal, &zelf.counter, |deque, pos| {
                    deque
                        .len()
                        .checked_sub(pos + 1)
                        .and_then(|pos| deque.get(pos))
                        .cloned()
                })
            })
        }
    }
    #[pyattr]
    #[pyclass(
        module = "collections",
        name = "defaultdict",
        base = PyDict,
        unhashable = true,
        traverse = "manual"
    )]
    #[derive(Debug, Default)]
    struct PyDefaultDict {
        dict: PyDict,
        default_factory: PyRwLock<Option<PyObjectRef>>,
    }

    // SAFETY: Traverse visits each owned Python reference at most once.
    unsafe impl Traverse for PyDefaultDict {
        fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
            self.dict.traverse(tracer_fn);
            self.default_factory.traverse(tracer_fn);
        }

        fn clear(&mut self, out: &mut Vec<PyObjectRef>) {
            Traverse::clear(&mut self.dict, out);
            if let Some(factory) = self.default_factory.get_mut().take() {
                out.push(factory);
            }
        }
    }

    #[pyclass(
        with(AsMapping, AsNumber, Constructor, Initializer, Representable),
        flags(BASETYPE, MAPPING, HAS_DICT)
    )]
    impl PyDefaultDict {
        #[pygetset]
        fn default_factory(&self) -> Option<PyObjectRef> {
            self.default_factory.read().clone()
        }

        #[pygetset(name = "default_factory", setter)]
        fn default_factory_setter(&self, value: PyObjectRef, vm: &VirtualMachine) {
            *self.default_factory.write() = if value.is(&vm.ctx.none()) {
                None
            } else {
                Some(value)
            };
        }

        #[pymethod]
        fn __missing__(&self, key: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            let factory = self.default_factory();

            if let Some(f) = factory {
                let value = f.call((), vm)?;
                self.dict.setdefault(key, value.into(), vm)
            } else {
                Err(vm.new_key_error(key))
            }
        }

        #[pymethod]
        #[pymethod(name = "__copy__")]
        fn copy(&self) -> Self {
            let default_factory = self.default_factory();

            Self {
                dict: self.dict.copy(),
                default_factory: PyRwLock::new(default_factory),
            }
        }

        #[pymethod]
        fn __reduce__(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            let cls = zelf.class().to_owned();

            let default_factory = zelf.default_factory();
            let factory_tuple_elements =
                default_factory.map_or_else(Vec::new, |factory| vec![factory]);
            let factory_tuple = vm.ctx.new_tuple(factory_tuple_elements);

            let items_fn = zelf.as_object().get_attr("items", vm)?;
            let items_iter = items_fn.call((), vm)?;
            let iter = items_iter.get_iter(vm)?;
            let none = vm.ctx.none();

            Ok(vm
                .ctx
                .new_tuple(vec![
                    cls.into(),
                    factory_tuple.into(),
                    none.clone(),
                    none,
                    iter.into(),
                ])
                .into())
        }
    }

    impl PyDefaultDict {
        fn __or__(lhs: PyObjectRef, rhs: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            let not_implemented = || Ok(vm.ctx.not_implemented.clone().into());

            let (default_factory, dict) = if let Some(zelf) = lhs.downcast_ref::<Self>() {
                if !rhs.fast_isinstance(vm.ctx.types.dict_type) {
                    return not_implemented();
                }

                (zelf.default_factory(), zelf.dict.copy())
            } else if let Some(zelf) = rhs.downcast_ref::<Self>() {
                let Some(dict) = lhs.downcast_ref::<PyDict>() else {
                    return not_implemented();
                };

                (zelf.default_factory(), dict.copy())
            } else {
                return Err(vm.new_type_error(format!(
                    "unsupported operand type(s) for |: '{}' and '{}'",
                    lhs.class().name(),
                    rhs.class().name()
                )));
            };

            dict.update(rhs.into(), KwArgs::default(), vm)?;

            Ok(Self {
                dict,
                default_factory: PyRwLock::new(default_factory),
            }
            .to_pyobject(vm))
        }
    }

    impl DefaultConstructor for PyDefaultDict {}

    impl Initializer for PyDefaultDict {
        type Args = FuncArgs;

        fn init(zelf: PyRef<Self>, mut args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
            let default_factory = args.take_positional().map_or(Ok(None), |factory| {
                let is_none = factory.is(&vm.ctx.none());

                if !is_none && !factory.is_callable() {
                    Err(vm.new_type_error("first argument must be callable or None"))
                } else if is_none {
                    Ok(None)
                } else {
                    Ok(Some(factory))
                }
            })?;

            *zelf.default_factory.write() = default_factory;

            zelf.dict.update(
                OptionalArg::from_option(args.take_positional()),
                args.kwargs,
                vm,
            )?;

            Ok(())
        }
    }

    impl Representable for PyDefaultDict {
        fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
            let default_factory = zelf.default_factory.read();

            let factory_repr = match default_factory.as_ref() {
                Some(factory) => {
                    if let Some(_guard) = ReprGuard::enter(vm, factory) {
                        factory.repr(vm)?.to_string()
                    } else {
                        String::from("...")
                    }
                }
                None => String::from("None"),
            };

            let dict_repr = Representable::repr(&zelf.dict.copy().into_ref(&vm.ctx), vm)?;

            Ok(format!(
                "{}({}, {})",
                zelf.class().name(),
                factory_repr,
                dict_repr
            ))
        }
    }

    impl AsMapping for PyDefaultDict {
        fn as_mapping() -> &'static PyMappingMethods {
            PyDict::as_mapping()
        }
    }

    impl AsNumber for PyDefaultDict {
        fn as_number() -> &'static PyNumberMethods {
            static AS_NUMBER: PyNumberMethods = PyNumberMethods {
                or: Some(|a, b, vm| {
                    PyDefaultDict::__or__(a.to_pyobject(vm), b.to_pyobject(vm), vm)
                }),
                ..PyNumberMethods::NOT_IMPLEMENTED
            };
            &AS_NUMBER
        }
    }
    // ============================================================================
    // OrderedDict implementation
    // ============================================================================

    #[pyattr]
    #[pyclass(
        module = "_collections",
        name = "OrderedDict",
        base = PyDict,
        unhashable = true,
        traverse = "manual"
    )]
    #[derive(Debug, Default)]
    struct PyOrderedDict {
        inner: PyDict,
    }

    type PyOrderedDictRef = PyRef<PyOrderedDict>;

    // SAFETY: Traverse visits each owned Python reference at most once.
    unsafe impl Traverse for PyOrderedDict {
        fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
            self.inner.traverse(tracer_fn);
        }

        fn clear(&mut self, out: &mut Vec<PyObjectRef>) {
            Traverse::clear(&mut self.inner, out);
        }
    }

    #[derive(FromArgs)]
    struct MoveToEndArgs {
        #[pyarg(positional)]
        key: PyObjectRef,
        #[pyarg(any, default = true)]
        last: bool,
    }

    #[derive(FromArgs)]
    struct PopItemArgs {
        #[pyarg(any, default = true)]
        last: bool,
    }

    #[derive(FromArgs)]
    struct SetDefaultArgs {
        #[pyarg(positional)]
        key: PyObjectRef,
        #[pyarg(any, optional)]
        default: OptionalArg<PyObjectRef>,
    }

    #[derive(FromArgs)]
    struct PopArgs {
        #[pyarg(positional)]
        key: PyObjectRef,
        #[pyarg(any, optional)]
        default: OptionalArg<PyObjectRef>,
    }

    #[derive(FromArgs)]
    struct FromKeysArgs {
        #[pyarg(positional)]
        iterable: ArgIterable,
        #[pyarg(any, optional)]
        value: OptionalArg<PyObjectRef>,
    }

    #[pyclass(
        flags(BASETYPE, MAPPING, HAS_DICT),
        with(
            Constructor,
            Initializer,
            Comparable,
            Iterable,
            AsMapping,
            AsSequence,
            AsNumber,
            Representable
        )
    )]
    impl PyOrderedDict {
        /// Move an existing element to the end (or beginning if last is false).
        #[pymethod]
        fn move_to_end(&self, args: MoveToEndArgs, vm: &VirtualMachine) -> PyResult<()> {
            let entries = self.inner._as_dict_inner();
            if entries.move_to_end(vm, &*args.key, args.last)? {
                Ok(())
            } else {
                Err(vm.new_key_error(args.key))
            }
        }

        /// Remove and return a (key, value) pair from the dictionary.
        /// Pairs are returned in LIFO order if last is true or FIFO order if false.
        #[pymethod]
        fn popitem(
            &self,
            args: PopItemArgs,
            vm: &VirtualMachine,
        ) -> PyResult<(PyObjectRef, PyObjectRef)> {
            let entries = self.inner._as_dict_inner();
            let result = if args.last {
                entries.pop_back() // LIFO - existing method
            } else {
                entries.pop_front() // FIFO - new method
            };
            result.ok_or_else(|| {
                let err_msg = vm.ctx.new_str(ascii!("dictionary is empty")).into();
                vm.new_key_error(err_msg)
            })
        }

        #[pymethod]
        fn setdefault(&self, args: SetDefaultArgs, vm: &VirtualMachine) -> PyResult {
            self.inner
                ._as_dict_inner()
                .setdefault(vm, &*args.key, || args.default.unwrap_or_none(vm))
        }

        #[pymethod]
        fn pop(&self, args: PopArgs, vm: &VirtualMachine) -> PyResult {
            match self.inner._as_dict_inner().pop(vm, &*args.key)? {
                Some(value) => Ok(value),
                None => args.default.ok_or_else(|| vm.new_key_error(args.key)),
            }
        }

        #[pymethod]
        fn get(
            &self,
            key: PyObjectRef,
            default: OptionalArg<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult {
            match self.inner._as_dict_inner().get(vm, &*key)? {
                Some(value) => Ok(value),
                None => Ok(default.unwrap_or_none(vm)),
            }
        }

        #[pymethod]
        fn update(
            &self,
            dict_obj: OptionalArg<PyObjectRef>,
            kwargs: KwArgs,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            if let OptionalArg::Present(dict_obj) = dict_obj {
                self.inner.merge_object(dict_obj, vm)?;
            }
            for (key, value) in kwargs {
                self.inner._as_dict_inner().insert(vm, &key, value)?;
            }
            Ok(())
        }

        #[pymethod]
        fn clear(&self) {
            self.inner._as_dict_inner().clear()
        }

        #[pymethod(name = "__copy__")]
        #[pymethod]
        fn copy(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
            let new_inner = zelf.inner.copy();
            let new_ref =
                Self { inner: new_inner }.into_ref_with_type(vm, zelf.class().to_owned())?;

            // Copy instance __dict__ if it exists
            if let Some(inst_dict) = zelf.as_object().dict()
                && let Some(new_dict) = new_ref.as_object().dict()
            {
                for (key, value) in inst_dict.items_vec() {
                    new_dict._as_dict_inner().insert(vm, &*key, value)?;
                }
            }

            // Copy slot values using copyreg._slotnames
            if let Ok(copyreg) = vm.import("copyreg", 0)
                && let Ok(slotnames_func) = copyreg.get_attr("_slotnames", vm)
                && let Ok(slot_names) = slotnames_func.call((zelf.class().to_owned(),), vm)
                && let Ok(slot_list) = slot_names.downcast::<crate::builtins::PyList>()
            {
                // Collect slot names to avoid lifetime issues
                let names: Vec<String> = slot_list
                    .borrow_vec()
                    .iter()
                    .filter_map(|name| {
                        name.downcast_ref::<crate::builtins::PyStr>()
                            .map(ToString::to_string)
                    })
                    .filter(|s| s != "__dict__" && s != "__weakref__")
                    .collect();

                for name in names {
                    let interned = vm.ctx.intern_str(name.as_str());
                    if let Ok(value) = zelf.as_object().get_attr(interned, vm) {
                        let _ = new_ref.as_object().set_attr(interned, value, vm);
                    }
                }
            }

            Ok(new_ref)
        }

        #[pyclassmethod]
        fn fromkeys(class: PyTypeRef, args: FromKeysArgs, vm: &VirtualMachine) -> PyResult {
            let value = args.value.unwrap_or_none(vm);
            let d = PyType::call(&class, ().into(), vm)?;
            match d.downcast_exact::<Self>(vm) {
                Ok(ordered_dict) => {
                    for key in args.iterable.iter(vm)? {
                        let key: PyObjectRef = key?;
                        ordered_dict
                            .inner
                            ._as_dict_inner()
                            .insert(vm, &*key, value.clone())?;
                    }
                    Ok(ordered_dict.into_pyref().into())
                }
                Err(pyobj) => {
                    for key in args.iterable.iter(vm)? {
                        let key: PyObjectRef = key?;
                        pyobj.set_item(&*key, value.clone(), vm)?;
                    }
                    Ok(pyobj)
                }
            }
        }

        #[pymethod]
        fn __sizeof__(&self) -> usize {
            // Add overhead for OrderedDict's conceptual linked-list structure
            // In CPython, each entry has an additional _ODictNode with prev/next pointers
            let base_size = core::mem::size_of::<Self>() + self.inner._as_dict_inner().sizeof();
            // Add overhead: 2 pointers (prev, next) per entry + head/tail pointers
            let num_entries = self.inner._as_dict_inner().len();
            let pointer_size = core::mem::size_of::<usize>();
            let linked_list_overhead = 2 * pointer_size + num_entries * 2 * pointer_size;
            base_size + linked_list_overhead
        }

        /// Return a reverse iterator over the dict keys.
        #[pymethod]
        fn __reversed__(zelf: PyRef<Self>) -> PyOrderedDictReverseKeyIterator {
            PyOrderedDictReverseKeyIterator::new(zelf)
        }

        /// Return the OrderedDict keys view.
        #[pymethod]
        fn keys(zelf: PyRef<Self>) -> PyOrderedDictKeys {
            PyOrderedDictKeys { ordered_dict: zelf }
        }

        /// Return the OrderedDict values view.
        #[pymethod]
        fn values(zelf: PyRef<Self>) -> PyOrderedDictValues {
            PyOrderedDictValues { ordered_dict: zelf }
        }

        /// Return the OrderedDict items view.
        #[pymethod]
        fn items(zelf: PyRef<Self>) -> PyOrderedDictItems {
            PyOrderedDictItems { ordered_dict: zelf }
        }

        #[pymethod]
        fn __reduce__(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyObjectRef {
            // Return (OrderedDict, (list(self.items()),), state)
            // state can be (dict_state, slot_state) tuple or just dict_state
            let items: Vec<PyObjectRef> = zelf
                .inner
                ._as_dict_inner()
                .items()
                .into_iter()
                .map(|(k, v)| vm.new_tuple((k, v)).into())
                .collect();
            let items_list = vm.ctx.new_list(items);

            // Get instance __dict__ if it exists
            let inst_dict = zelf.as_object().dict();
            let dict_state: PyObjectRef = inst_dict
                .filter(|d| d.__len__() > 0)
                .map_or_else(|| vm.ctx.none(), |dict| dict.into());

            // Get slot state using copyreg._slotnames
            let mut slot_state: Option<PyObjectRef> = None;
            if let Ok(copyreg) = vm.import("copyreg", 0)
                && let Ok(slotnames_func) = copyreg.get_attr("_slotnames", vm)
                && let Ok(slot_names) = slotnames_func.call((zelf.class().to_owned(),), vm)
                && let Ok(slot_list) = slot_names.downcast::<crate::builtins::PyList>()
            {
                // Collect slot names to avoid lifetime issues
                let names: Vec<String> = slot_list
                    .borrow_vec()
                    .iter()
                    .filter_map(|name| {
                        name.downcast_ref::<crate::builtins::PyStr>()
                            .map(ToString::to_string)
                    })
                    .filter(|s| s != "__dict__" && s != "__weakref__")
                    .collect();

                let slots_dict = vm.ctx.new_dict();
                for name in names {
                    let interned = vm.ctx.intern_str(name.as_str());
                    if let Ok(value) = zelf.as_object().get_attr(interned, vm) {
                        let _ = slots_dict.set_item(name.as_str(), value, vm);
                    }
                }
                if !slots_dict.is_empty() {
                    slot_state = Some(slots_dict.into());
                }
            }

            // Construct final state
            let state: PyObjectRef = if let Some(slots) = slot_state {
                // Return (dict_state, slot_state) tuple
                vm.new_tuple((dict_state, slots)).into()
            } else {
                dict_state
            };

            vm.new_tuple((zelf.class().to_owned(), vm.new_tuple((items_list,)), state))
                .into()
        }

        #[pyclassmethod]
        fn __class_getitem__(
            cls: PyTypeRef,
            args: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyGenericAlias> {
            PyGenericAlias::from_args(cls, args, vm)
        }
    }

    impl DefaultConstructor for PyOrderedDict {}

    impl Initializer for PyOrderedDict {
        type Args = (OptionalArg<PyObjectRef>, KwArgs);

        fn init(
            zelf: PyRef<Self>,
            (dict_obj, kwargs): Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            // Do NOT clear existing data - just merge/update
            // This matches CPython behavior where __init__ updates existing dict
            // rather than replacing it

            // First add positional argument
            if let OptionalArg::Present(dict_obj) = dict_obj {
                zelf.inner.merge_object(dict_obj, vm)?;
            }

            // Then add keyword arguments (in order)
            for (key, value) in kwargs {
                zelf.inner._as_dict_inner().insert(vm, &key, value)?;
            }

            Ok(())
        }
    }

    impl Comparable for PyOrderedDict {
        fn cmp(
            zelf: &Py<Self>,
            other: &PyObject,
            op: PyComparisonOp,
            vm: &VirtualMachine,
        ) -> PyResult<PyComparisonValue> {
            // Check for identity optimization
            if let Some(res) = op.identical_optimization(zelf, other) {
                return Ok(res.into());
            }

            // Order-sensitive comparison when comparing two OrderedDicts
            if let Some(other_ordered_dict) = other.downcast_ref::<Self>()
                && (op == PyComparisonOp::Eq || op == PyComparisonOp::Ne)
            {
                let self_items = zelf.inner._as_dict_inner().items();
                let other_items = other_ordered_dict.inner._as_dict_inner().items();

                if self_items.len() != other_items.len() {
                    return Ok(PyComparisonValue::Implemented(op == PyComparisonOp::Ne));
                }

                for ((k1, v1), (k2, v2)) in self_items.iter().zip(other_items.iter()) {
                    // Check keys are equal and in same order
                    if !vm.identical_or_equal(k1, k2)? {
                        return Ok(PyComparisonValue::Implemented(op == PyComparisonOp::Ne));
                    }
                    // Check values are equal
                    if !vm.identical_or_equal(v1, v2)? {
                        return Ok(PyComparisonValue::Implemented(op == PyComparisonOp::Ne));
                    }
                }
                return Ok(PyComparisonValue::Implemented(op == PyComparisonOp::Eq));
            }

            // Fall back to dict comparison (order-insensitive) for other types
            if let Some(other_dict) = other.downcast_ref::<PyDict>() {
                op.eq_only(|| {
                    let self_entries = zelf.inner._as_dict_inner();
                    let other_entries = other_dict._as_dict_inner();

                    if self_entries.len() != other_entries.len() {
                        return Ok(PyComparisonValue::Implemented(false));
                    }

                    for (k, v1) in self_entries.items() {
                        match other_entries.get(vm, &*k)? {
                            Some(v2) => {
                                if !vm.identical_or_equal(&v1, &v2)? {
                                    return Ok(PyComparisonValue::Implemented(false));
                                }
                            }
                            None => return Ok(PyComparisonValue::Implemented(false)),
                        }
                    }
                    Ok(PyComparisonValue::Implemented(true))
                })
            } else {
                Ok(PyComparisonValue::NotImplemented)
            }
        }
    }

    impl Iterable for PyOrderedDict {
        fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            Ok(PyOrderedDictKeyIterator::new(zelf).into_pyobject(vm))
        }
    }

    impl AsMapping for PyOrderedDict {
        fn as_mapping() -> &'static PyMappingMethods {
            static AS_MAPPING: PyMappingMethods = PyMappingMethods {
                length: atomic_func!(|mapping, _vm| Ok(PyOrderedDict::mapping_downcast(mapping)
                    .inner
                    ._as_dict_inner()
                    .len())),
                subscript: atomic_func!(|mapping, needle, vm| {
                    let zelf = PyOrderedDict::mapping_downcast(mapping);
                    match zelf.inner._as_dict_inner().get(vm, needle)? {
                        Some(value) => Ok(value),
                        None => Err(vm.new_key_error(needle.to_owned())),
                    }
                }),
                ass_subscript: atomic_func!(|mapping, needle, value, vm| {
                    let zelf = PyOrderedDict::mapping_downcast(mapping);
                    if let Some(value) = value {
                        zelf.inner._as_dict_inner().insert(vm, needle, value)
                    } else {
                        zelf.inner._as_dict_inner().delete(vm, needle)
                    }
                }),
            };
            &AS_MAPPING
        }
    }

    impl AsSequence for PyOrderedDict {
        fn as_sequence() -> &'static PySequenceMethods {
            static AS_SEQUENCE: PySequenceMethods = PySequenceMethods {
                contains: atomic_func!(|seq, target, vm| PyOrderedDict::sequence_downcast(seq)
                    .inner
                    ._as_dict_inner()
                    .contains(vm, target)),
                ..PySequenceMethods::NOT_IMPLEMENTED
            };
            &AS_SEQUENCE
        }
    }

    impl AsNumber for PyOrderedDict {
        fn as_number() -> &'static PyNumberMethods {
            static AS_NUMBER: PyNumberMethods = PyNumberMethods {
                // Handle both __or__ and __ror__ in the same function
                // This function is used for both `or` and `right_or` slots via copy_from
                or: Some(|a, b, vm| {
                    let a_is_ordered_dict = a.downcast_ref::<PyOrderedDict>().is_some();
                    let b_is_ordered_dict = b.downcast_ref::<PyOrderedDict>().is_some();
                    let a_is_dict = a.class().fast_issubclass(vm.ctx.types.dict_type);
                    let b_is_dict = b.class().fast_issubclass(vm.ctx.types.dict_type);

                    if a_is_ordered_dict {
                        // This is __or__: OrderedDict | other
                        // other must be a dict or dict subclass
                        if !b_is_dict {
                            return Ok(vm.ctx.not_implemented());
                        }
                        let a_ordered_dict = a.downcast_ref::<PyOrderedDict>().unwrap();
                        let new_inner = a_ordered_dict.inner.copy();
                        new_inner.merge_object(b.to_pyobject(vm), vm)?;
                        // Preserve the subclass type (use a's type)
                        let result = PyOrderedDict { inner: new_inner }
                            .into_ref_with_type(vm, a.class().to_owned())?;
                        Ok(result.into())
                    } else if b_is_ordered_dict {
                        // This is __ror__: other | OrderedDict
                        // other must be a dict or dict subclass
                        if !a_is_dict {
                            return Ok(vm.ctx.not_implemented());
                        }
                        let b_ordered_dict = b.downcast_ref::<PyOrderedDict>().unwrap();
                        // Create new instance with b's type (preserve subclass)
                        let new_inner = PyDict::default();
                        new_inner.merge_object(a.to_pyobject(vm), vm)?;
                        for (key, value) in b_ordered_dict.inner._as_dict_inner().items() {
                            new_inner._as_dict_inner().insert(vm, &*key, value)?;
                        }
                        let result = PyOrderedDict { inner: new_inner }
                            .into_ref_with_type(vm, b.class().to_owned())?;
                        Ok(result.into())
                    } else {
                        Ok(vm.ctx.not_implemented())
                    }
                }),
                inplace_or: Some(|a, b, vm| {
                    if let Some(a) = a.downcast_ref::<PyOrderedDict>() {
                        a.inner.merge_object(b.to_pyobject(vm), vm)?;
                        Ok(a.to_owned().into())
                    } else {
                        Ok(vm.ctx.not_implemented())
                    }
                }),
                ..PyNumberMethods::NOT_IMPLEMENTED
            };
            &AS_NUMBER
        }
    }

    impl Representable for PyOrderedDict {
        #[inline]
        fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
            let class = zelf.class();
            let class_name = class.name();

            if zelf.inner._as_dict_inner().len() == 0 {
                return Ok(format!("{class_name}()"));
            }

            if let Some(_guard) = ReprGuard::enter(vm, zelf.as_object()) {
                let mut str_parts = Vec::with_capacity(zelf.inner._as_dict_inner().len());
                for (key, value) in zelf.inner._as_dict_inner().items() {
                    let key_repr: PyStrRef = key.repr(vm)?;
                    let value_repr: PyStrRef = value.repr(vm)?;
                    str_parts.push(format!("{key_repr}: {value_repr}"));
                }
                Ok(format!("{class_name}({{{}}})", str_parts.join(", ")))
            } else {
                // Recursion detected - return just "..." as CPython does
                Ok("...".to_owned())
            }
        }
    }

    // ============================================================================
    // OrderedDict Views
    // ============================================================================

    #[pyattr]
    #[pyclass(module = "_collections", name = "odict_keys")]
    #[derive(Debug, PyPayload)]
    struct PyOrderedDictKeys {
        ordered_dict: PyOrderedDictRef,
    }

    #[pyclass(with(Iterable, Comparable, AsSequence, Representable))]
    impl PyOrderedDictKeys {
        #[pymethod]
        fn __reversed__(&self) -> PyOrderedDictReverseKeyIterator {
            PyOrderedDictReverseKeyIterator::new(self.ordered_dict.clone())
        }
    }

    impl AsSequence for PyOrderedDictKeys {
        fn as_sequence() -> &'static PySequenceMethods {
            static AS_SEQUENCE: PySequenceMethods = PySequenceMethods {
                length: atomic_func!(|seq, _vm| Ok(PyOrderedDictKeys::sequence_downcast(seq)
                    .ordered_dict
                    .inner
                    ._as_dict_inner()
                    .len())),
                contains: atomic_func!(|seq, target, vm| PyOrderedDictKeys::sequence_downcast(seq)
                    .ordered_dict
                    .inner
                    ._as_dict_inner()
                    .contains(vm, target)),
                ..PySequenceMethods::NOT_IMPLEMENTED
            };
            &AS_SEQUENCE
        }
    }

    impl Iterable for PyOrderedDictKeys {
        fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            Ok(PyOrderedDictKeyIterator::new(zelf.ordered_dict.clone()).into_pyobject(vm))
        }
    }

    impl Comparable for PyOrderedDictKeys {
        fn cmp(
            zelf: &Py<Self>,
            other: &PyObject,
            op: PyComparisonOp,
            vm: &VirtualMachine,
        ) -> PyResult<PyComparisonValue> {
            // Convert both to lists for comparison (like CPython)
            let self_keys: Vec<PyObjectRef> = zelf.ordered_dict.inner._as_dict_inner().keys();
            let other_vec: Result<Vec<PyObjectRef>, _> = other.try_to_value(vm);

            if let Ok(other_keys) = other_vec {
                let other_keys: &Vec<PyObjectRef> = &other_keys;
                self_keys
                    .iter()
                    .richcompare(other_keys.iter(), op, vm)
                    .map(PyComparisonValue::Implemented)
            } else {
                Ok(PyComparisonValue::NotImplemented)
            }
        }
    }

    impl Representable for PyOrderedDictKeys {
        #[inline]
        fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
            if let Some(_guard) = ReprGuard::enter(vm, zelf.as_object()) {
                let mut str_parts =
                    Vec::with_capacity(zelf.ordered_dict.inner._as_dict_inner().len());
                for key in zelf.ordered_dict.inner._as_dict_inner().keys() {
                    let repr: PyStrRef = key.repr(vm)?;
                    str_parts.push(repr.to_string());
                }
                Ok(format!("odict_keys([{}])", str_parts.join(", ")))
            } else {
                Ok("odict_keys(...)".to_owned())
            }
        }
    }

    #[pyattr]
    #[pyclass(module = "_collections", name = "odict_values")]
    #[derive(Debug, PyPayload)]
    struct PyOrderedDictValues {
        ordered_dict: PyOrderedDictRef,
    }

    #[pyclass(with(Iterable, AsSequence, Representable))]
    impl PyOrderedDictValues {
        #[pymethod]
        fn __reversed__(&self) -> PyOrderedDictReverseValueIterator {
            PyOrderedDictReverseValueIterator::new(self.ordered_dict.clone())
        }
    }

    impl AsSequence for PyOrderedDictValues {
        fn as_sequence() -> &'static PySequenceMethods {
            static AS_SEQUENCE: PySequenceMethods = PySequenceMethods {
                length: atomic_func!(|seq, _vm| Ok(PyOrderedDictValues::sequence_downcast(seq)
                    .ordered_dict
                    .inner
                    ._as_dict_inner()
                    .len())),
                ..PySequenceMethods::NOT_IMPLEMENTED
            };
            &AS_SEQUENCE
        }
    }

    impl Iterable for PyOrderedDictValues {
        fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            Ok(PyOrderedDictValueIterator::new(zelf.ordered_dict.clone()).into_pyobject(vm))
        }
    }

    impl Representable for PyOrderedDictValues {
        #[inline]
        fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
            if let Some(_guard) = ReprGuard::enter(vm, zelf.as_object()) {
                let mut str_parts =
                    Vec::with_capacity(zelf.ordered_dict.inner._as_dict_inner().len());
                for value in zelf.ordered_dict.inner._as_dict_inner().values() {
                    let repr: PyStrRef = value.repr(vm)?;
                    str_parts.push(repr.to_string());
                }
                Ok(format!("odict_values([{}])", str_parts.join(", ")))
            } else {
                Ok("odict_values(...)".to_owned())
            }
        }
    }

    #[pyattr]
    #[pyclass(module = "_collections", name = "odict_items")]
    #[derive(Debug, PyPayload)]
    struct PyOrderedDictItems {
        ordered_dict: PyOrderedDictRef,
    }

    #[pyclass(with(Iterable, Comparable, AsSequence, Representable))]
    impl PyOrderedDictItems {
        #[pymethod]
        fn __reversed__(&self) -> PyOrderedDictReverseItemIterator {
            PyOrderedDictReverseItemIterator::new(self.ordered_dict.clone())
        }
    }

    impl AsSequence for PyOrderedDictItems {
        fn as_sequence() -> &'static PySequenceMethods {
            static AS_SEQUENCE: PySequenceMethods = PySequenceMethods {
                length: atomic_func!(|seq, _vm| Ok(PyOrderedDictItems::sequence_downcast(seq)
                    .ordered_dict
                    .inner
                    ._as_dict_inner()
                    .len())),
                ..PySequenceMethods::NOT_IMPLEMENTED
            };
            &AS_SEQUENCE
        }
    }

    impl Iterable for PyOrderedDictItems {
        fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            Ok(PyOrderedDictItemIterator::new(zelf.ordered_dict.clone()).into_pyobject(vm))
        }
    }

    impl Comparable for PyOrderedDictItems {
        fn cmp(
            zelf: &Py<Self>,
            other: &PyObject,
            op: PyComparisonOp,
            vm: &VirtualMachine,
        ) -> PyResult<PyComparisonValue> {
            // Convert both to lists for comparison
            let self_items: Vec<PyObjectRef> = zelf
                .ordered_dict
                .inner
                ._as_dict_inner()
                .items()
                .into_iter()
                .map(|(k, v)| vm.new_tuple((k, v)).into())
                .collect();
            let other_vec: Result<Vec<PyObjectRef>, _> = other.try_to_value(vm);

            if let Ok(other_items) = other_vec {
                let other_items: &Vec<PyObjectRef> = &other_items;
                self_items
                    .iter()
                    .richcompare(other_items.iter(), op, vm)
                    .map(PyComparisonValue::Implemented)
            } else {
                Ok(PyComparisonValue::NotImplemented)
            }
        }
    }

    impl Representable for PyOrderedDictItems {
        #[inline]
        fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
            if let Some(_guard) = ReprGuard::enter(vm, zelf.as_object()) {
                let mut str_parts =
                    Vec::with_capacity(zelf.ordered_dict.inner._as_dict_inner().len());
                for (key, value) in zelf.ordered_dict.inner._as_dict_inner().items() {
                    let key_repr: PyStrRef = key.repr(vm)?;
                    let value_repr: PyStrRef = value.repr(vm)?;
                    str_parts.push(format!("({key_repr}, {value_repr})"));
                }
                Ok(format!("odict_items([{}])", str_parts.join(", ")))
            } else {
                Ok("odict_items(...)".to_owned())
            }
        }
    }

    // ============================================================================
    // OrderedDict Iterators
    // ============================================================================

    #[pyattr]
    #[pyclass(module = "_collections", name = "odict_keyiterator")]
    #[derive(Debug, PyPayload)]
    struct PyOrderedDictKeyIterator {
        size: dict_inner::DictSize,
        internal: PyMutex<PositionIterInternal<PyOrderedDictRef>>,
    }

    impl PyOrderedDictKeyIterator {
        fn new(ordered_dict: PyOrderedDictRef) -> Self {
            let size = ordered_dict.inner._as_dict_inner().size();
            Self {
                size,
                internal: PyMutex::new(PositionIterInternal::new(ordered_dict, 0)),
            }
        }
    }

    #[pyclass(with(IterNext, Iterable))]
    impl PyOrderedDictKeyIterator {
        #[pymethod]
        fn __length_hint__(&self) -> usize {
            self.internal.lock().length_hint(|_| self.size.entries_size)
        }
    }

    impl SelfIter for PyOrderedDictKeyIterator {}
    impl IterNext for PyOrderedDictKeyIterator {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            locked_step(&zelf.internal, |internal| {
                let Active(ordered_dict) = &internal.status else {
                    return (Ok(PyIterReturn::StopIteration(None)), None);
                };
                match ordered_dict.inner._as_dict_inner().next_entry_checked(
                    internal.position,
                    &zelf.size,
                    |key, _value| key.clone(),
                ) {
                    Err(dict_inner::DictChanged) => (
                        Err(vm.new_runtime_error("dictionary changed size during iteration")),
                        internal.exhaust(),
                    ),
                    Ok(Some((position, key))) => {
                        internal.position = position;
                        (Ok(PyIterReturn::Return(key)), None)
                    }
                    Ok(None) => (Ok(PyIterReturn::StopIteration(None)), internal.exhaust()),
                }
            })
        }
    }

    #[pyattr]
    #[pyclass(module = "_collections", name = "odict_valueiterator")]
    #[derive(Debug, PyPayload)]
    struct PyOrderedDictValueIterator {
        size: dict_inner::DictSize,
        internal: PyMutex<PositionIterInternal<PyOrderedDictRef>>,
    }

    impl PyOrderedDictValueIterator {
        fn new(ordered_dict: PyOrderedDictRef) -> Self {
            let size = ordered_dict.inner._as_dict_inner().size();
            Self {
                size,
                internal: PyMutex::new(PositionIterInternal::new(ordered_dict, 0)),
            }
        }
    }

    #[pyclass(with(IterNext, Iterable))]
    impl PyOrderedDictValueIterator {
        #[pymethod]
        fn __length_hint__(&self) -> usize {
            self.internal.lock().length_hint(|_| self.size.entries_size)
        }
    }

    impl SelfIter for PyOrderedDictValueIterator {}
    impl IterNext for PyOrderedDictValueIterator {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            locked_step(&zelf.internal, |internal| {
                let Active(ordered_dict) = &internal.status else {
                    return (Ok(PyIterReturn::StopIteration(None)), None);
                };
                match ordered_dict.inner._as_dict_inner().next_entry_checked(
                    internal.position,
                    &zelf.size,
                    |_key, value| value.clone(),
                ) {
                    Err(dict_inner::DictChanged) => (
                        Err(vm.new_runtime_error("dictionary changed size during iteration")),
                        internal.exhaust(),
                    ),
                    Ok(Some((position, value))) => {
                        internal.position = position;
                        (Ok(PyIterReturn::Return(value)), None)
                    }
                    Ok(None) => (Ok(PyIterReturn::StopIteration(None)), internal.exhaust()),
                }
            })
        }
    }

    #[pyattr]
    #[pyclass(module = "_collections", name = "odict_itemiterator")]
    #[derive(Debug, PyPayload)]
    struct PyOrderedDictItemIterator {
        size: dict_inner::DictSize,
        internal: PyMutex<PositionIterInternal<PyOrderedDictRef>>,
    }

    impl PyOrderedDictItemIterator {
        fn new(ordered_dict: PyOrderedDictRef) -> Self {
            let size = ordered_dict.inner._as_dict_inner().size();
            Self {
                size,
                internal: PyMutex::new(PositionIterInternal::new(ordered_dict, 0)),
            }
        }
    }

    #[pyclass(with(IterNext, Iterable))]
    impl PyOrderedDictItemIterator {
        #[pymethod]
        fn __length_hint__(&self) -> usize {
            self.internal.lock().length_hint(|_| self.size.entries_size)
        }
    }

    impl SelfIter for PyOrderedDictItemIterator {}
    impl IterNext for PyOrderedDictItemIterator {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            locked_step(&zelf.internal, |internal| {
                let Active(ordered_dict) = &internal.status else {
                    return (Ok(PyIterReturn::StopIteration(None)), None);
                };
                match ordered_dict.inner._as_dict_inner().next_entry_checked(
                    internal.position,
                    &zelf.size,
                    |key, value| (key.clone(), value.clone()),
                ) {
                    Err(dict_inner::DictChanged) => (
                        Err(vm.new_runtime_error("dictionary changed size during iteration")),
                        internal.exhaust(),
                    ),
                    Ok(Some((position, (key, value)))) => {
                        internal.position = position;
                        (
                            Ok(PyIterReturn::Return(vm.new_tuple((key, value)).into())),
                            None,
                        )
                    }
                    Ok(None) => (Ok(PyIterReturn::StopIteration(None)), internal.exhaust()),
                }
            })
        }
    }

    // Reverse iterators

    #[pyattr]
    #[pyclass(module = "_collections", name = "odict_reverse_keyiterator")]
    #[derive(Debug, PyPayload)]
    struct PyOrderedDictReverseKeyIterator {
        size: dict_inner::DictSize,
        internal: PyMutex<PositionIterInternal<PyOrderedDictRef>>,
    }

    impl PyOrderedDictReverseKeyIterator {
        fn new(ordered_dict: PyOrderedDictRef) -> Self {
            let size = ordered_dict.inner._as_dict_inner().size();
            let position = size.entries_size.saturating_sub(1);
            Self {
                size,
                internal: PyMutex::new(PositionIterInternal::new(ordered_dict, position)),
            }
        }
    }

    #[pyclass(with(IterNext, Iterable))]
    impl PyOrderedDictReverseKeyIterator {
        #[pymethod]
        fn __length_hint__(&self) -> usize {
            self.internal
                .lock()
                .rev_length_hint(|_| self.size.entries_size)
        }
    }

    impl SelfIter for PyOrderedDictReverseKeyIterator {}
    impl IterNext for PyOrderedDictReverseKeyIterator {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            locked_step(&zelf.internal, |internal| {
                let Active(ordered_dict) = &internal.status else {
                    return (Ok(PyIterReturn::StopIteration(None)), None);
                };
                match ordered_dict.inner._as_dict_inner().prev_entry_checked(
                    internal.position,
                    &zelf.size,
                    |key, _value| key.clone(),
                ) {
                    Err(dict_inner::DictChanged) => (
                        Err(vm.new_runtime_error("dictionary changed size during iteration")),
                        internal.exhaust(),
                    ),
                    Ok(Some((position, key))) => {
                        let released = if position == 0 {
                            internal.exhaust()
                        } else {
                            internal.position = position - 1;
                            None
                        };
                        (Ok(PyIterReturn::Return(key)), released)
                    }
                    Ok(None) => (Ok(PyIterReturn::StopIteration(None)), internal.exhaust()),
                }
            })
        }
    }

    #[pyattr]
    #[pyclass(module = "_collections", name = "odict_reverse_valueiterator")]
    #[derive(Debug, PyPayload)]
    struct PyOrderedDictReverseValueIterator {
        size: dict_inner::DictSize,
        internal: PyMutex<PositionIterInternal<PyOrderedDictRef>>,
    }

    impl PyOrderedDictReverseValueIterator {
        fn new(ordered_dict: PyOrderedDictRef) -> Self {
            let size = ordered_dict.inner._as_dict_inner().size();
            let position = size.entries_size.saturating_sub(1);
            Self {
                size,
                internal: PyMutex::new(PositionIterInternal::new(ordered_dict, position)),
            }
        }
    }

    #[pyclass(with(IterNext, Iterable))]
    impl PyOrderedDictReverseValueIterator {
        #[pymethod]
        fn __length_hint__(&self) -> usize {
            self.internal
                .lock()
                .rev_length_hint(|_| self.size.entries_size)
        }
    }

    impl SelfIter for PyOrderedDictReverseValueIterator {}
    impl IterNext for PyOrderedDictReverseValueIterator {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            locked_step(&zelf.internal, |internal| {
                let Active(ordered_dict) = &internal.status else {
                    return (Ok(PyIterReturn::StopIteration(None)), None);
                };
                match ordered_dict.inner._as_dict_inner().prev_entry_checked(
                    internal.position,
                    &zelf.size,
                    |_key, value| value.clone(),
                ) {
                    Err(dict_inner::DictChanged) => (
                        Err(vm.new_runtime_error("dictionary changed size during iteration")),
                        internal.exhaust(),
                    ),
                    Ok(Some((position, value))) => {
                        let released = if position == 0 {
                            internal.exhaust()
                        } else {
                            internal.position = position - 1;
                            None
                        };
                        (Ok(PyIterReturn::Return(value)), released)
                    }
                    Ok(None) => (Ok(PyIterReturn::StopIteration(None)), internal.exhaust()),
                }
            })
        }
    }

    #[pyattr]
    #[pyclass(module = "_collections", name = "odict_reverse_itemiterator")]
    #[derive(Debug, PyPayload)]
    struct PyOrderedDictReverseItemIterator {
        size: dict_inner::DictSize,
        internal: PyMutex<PositionIterInternal<PyOrderedDictRef>>,
    }

    impl PyOrderedDictReverseItemIterator {
        fn new(ordered_dict: PyOrderedDictRef) -> Self {
            let size = ordered_dict.inner._as_dict_inner().size();
            let position = size.entries_size.saturating_sub(1);
            Self {
                size,
                internal: PyMutex::new(PositionIterInternal::new(ordered_dict, position)),
            }
        }
    }

    #[pyclass(with(IterNext, Iterable))]
    impl PyOrderedDictReverseItemIterator {
        #[pymethod]
        fn __length_hint__(&self) -> usize {
            self.internal
                .lock()
                .rev_length_hint(|_| self.size.entries_size)
        }
    }

    impl SelfIter for PyOrderedDictReverseItemIterator {}
    impl IterNext for PyOrderedDictReverseItemIterator {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            locked_step(&zelf.internal, |internal| {
                let Active(ordered_dict) = &internal.status else {
                    return (Ok(PyIterReturn::StopIteration(None)), None);
                };
                match ordered_dict.inner._as_dict_inner().prev_entry_checked(
                    internal.position,
                    &zelf.size,
                    |key, value| (key.clone(), value.clone()),
                ) {
                    Err(dict_inner::DictChanged) => (
                        Err(vm.new_runtime_error("dictionary changed size during iteration")),
                        internal.exhaust(),
                    ),
                    Ok(Some((position, (key, value)))) => {
                        let released = if position == 0 {
                            internal.exhaust()
                        } else {
                            internal.position = position - 1;
                            None
                        };
                        (
                            Ok(PyIterReturn::Return(vm.new_tuple((key, value)).into())),
                            released,
                        )
                    }
                    Ok(None) => (Ok(PyIterReturn::StopIteration(None)), internal.exhaust()),
                }
            })
        }
    }
}
