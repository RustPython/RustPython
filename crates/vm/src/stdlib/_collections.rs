// cspell:ignore odict

pub(crate) use _collections::module_def;

pub(crate) mod ordered_dict;

#[pymodule(with(ordered_dict::ordered_dict))]
mod _collections {
    use crate::{
        AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject,
        VirtualMachine, atomic_func,
        builtins::{
            IterStatus::{Active, Exhausted},
            PositionIterInternal, PyDict, PyGenericAlias, PyInt, PyList, PyStr, PyTuple, PyType,
            PyTypeRef, locked_step,
        },
        common::lock::{PyMutex, PyRwLock, PyRwLockReadGuard, PyRwLockWriteGuard},
        convert::ToPyObject,
        function::{
            ArgIterable, FuncArgs, KwArgs, OptionalArg, PyComparisonValue, PySetterValue, PySsize,
        },
        object::{PyAtomicRef, Traverse, TraverseFn},
        protocol::{PyIter, PyIterReturn, PyMappingMethods, PyNumberMethods, PySequenceMethods},
        recursion::ReprGuard,
        sequence::{MutObjectSequenceOp, OptionalRangeArgs},
        sliceable::SequenceIndexOp,
        types::{
            AsMapping, AsNumber, AsSequence, Comparable, Constructor, DefaultConstructor,
            GetDescriptor, Initializer, IterNext, Iterable, PyComparisonOp, Representable,
            SelfIter,
        },
        utils::collection_repr,
        vm::MAX_MEMORY_SIZE,
    };
    use alloc::collections::VecDeque;
    use core::{cmp::max, mem::size_of};
    use crossbeam_utils::atomic::AtomicCell;

    #[derive(FromArgs)]
    struct RotateArgs {
        #[pyarg(positional, default = 1)]
        n: isize,
    }

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

        fn append_item(&self, item: PyObjectRef, left: bool, maxlen: Option<usize>) {
            let removed = {
                let mut deque = self.borrow_deque_mut();
                if left {
                    deque.push_front(item);
                } else {
                    deque.push_back(item);
                }
                self.state.fetch_add(1);
                // Trim after pushing, so that a `maxlen` of zero drops what just
                // arrived instead of popping from an empty deque and keeping it.
                if maxlen.is_some_and(|maxlen| deque.len() > maxlen) {
                    if left {
                        deque.pop_back()
                    } else {
                        deque.pop_front()
                    }
                } else {
                    None
                }
            };
            drop(removed);
        }

        fn extend_items(
            &self,
            items: impl Iterator<Item = PyResult>,
            left: bool,
            maxlen: Option<usize>,
        ) -> PyResult<()> {
            for item in items {
                let item = item?;
                if maxlen != Some(0) {
                    self.append_item(item, left, maxlen);
                }
            }
            Ok(())
        }
    }

    impl PyDeque {
        fn _extend(&self, iterable: &PyObject, left: bool, vm: &VirtualMachine) -> PyResult<()> {
            if iterable
                .downcast_ref::<Self>()
                .is_some_and(|other| core::ptr::eq(self, &**other))
            {
                // Snapshot self-extension through the iterator protocol so
                // subclass overrides of __iter__ are still respected.
                let elements: Vec<PyObjectRef> = iterable.try_to_value(vm)?;
                // Retain the snapshot until extension finishes, as a temporary list does.
                return self.extend_items(elements.iter().cloned().map(Ok), left, self.maxlen);
            }

            let maxlen = self.maxlen;
            let class = iterable.class();
            if maxlen.is_none()
                && (class.is(vm.ctx.types.list_type) || class.is(vm.ctx.types.tuple_type))
            {
                // Exact containers cannot run Python while being copied, and
                // an unbounded deque cannot evict items. Fill under one lock.
                let elements: Vec<PyObjectRef> = iterable.try_to_value(vm)?;
                let count = elements.len();
                if count != 0 {
                    let mut deque = self.borrow_deque_mut();
                    deque.reserve(count);
                    self.state.fetch_add(count);
                    if left {
                        for item in elements {
                            deque.push_front(item);
                        }
                    } else {
                        deque.extend(elements);
                    }
                }
                return Ok(());
            }

            if class.is(vm.ctx.types.tuple_type) {
                let tuple = iterable.downcast_ref::<PyTuple>().unwrap();
                self.extend_items(tuple.as_slice().iter().cloned().map(Ok), left, maxlen)
            } else if class.is(vm.ctx.types.list_type) {
                let list = iterable.downcast_ref::<PyList>().unwrap();
                let mut index = 0;
                let items = core::iter::from_fn(|| {
                    // Evicted elements can mutate the source list in __del__.
                    let item = list.borrow_vec().get(index).cloned();
                    index += 1;
                    item.map(Ok)
                });
                self.extend_items(items, left, maxlen)
            } else {
                self.extend_items(iterable.get_iter(vm)?.into_iter(vm), left, maxlen)
            }
        }

        fn __getitem__(&self, idx: isize, vm: &VirtualMachine) -> PyResult {
            let deque = self.borrow_deque();
            idx.wrapped_at(deque.len())
                .and_then(|i| deque.get(i).cloned())
                .ok_or_else(|| vm.new_index_error("deque index out of range"))
        }

        fn __setitem__(&self, idx: isize, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            let removed = {
                let mut deque = self.borrow_deque_mut();
                let item = idx
                    .wrapped_at(deque.len())
                    .and_then(|i| deque.get_mut(i))
                    .ok_or_else(|| vm.new_index_error("deque index out of range"))?;
                core::mem::replace(item, value)
            };
            drop(removed);
            Ok(())
        }

        fn __delitem__(&self, idx: isize, vm: &VirtualMachine) -> PyResult<()> {
            let removed = {
                let mut deque = self.borrow_deque_mut();
                let removed = idx
                    .wrapped_at(deque.len())
                    .and_then(|i| deque.remove(i))
                    .ok_or_else(|| vm.new_index_error("deque index out of range"))?;
                self.state.fetch_add(1);
                removed
            };
            // Finalizers can mutate the deque, so release the lock first.
            drop(removed);
            Ok(())
        }

        fn __contains__(&self, needle: &PyObject, vm: &VirtualMachine) -> PyResult<bool> {
            self._contains(needle, vm)
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
                return Err(vm.no_memory_error());
            }
            // A maxlen keeps only the last `result_len` items; start the cycle where they begin.
            let start = (mul_len - result_len).checked_rem(deque.len()).unwrap_or(0);
            let mut result = VecDeque::new();
            result
                .try_reserve_exact(result_len)
                .map_err(|_| vm.no_memory_error())?;
            result.extend(deque.iter().cycle().skip(start).take(result_len).cloned());
            Ok(result)
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
            if n == 1 || zelf.borrow_deque().is_empty() {
                return Ok(zelf);
            }
            let mut mul_deque = zelf._mul(n, vm)?;
            {
                let mut deque = zelf.borrow_deque_mut();
                core::mem::swap(&mut *deque, &mut mul_deque);
                zelf.state.fetch_add(1);
            }
            drop(mul_deque);
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
    impl Py<PyDeque> {
        #[pymethod]
        fn append(&self, item: PyObjectRef) {
            self.append_item(item, false, self.maxlen);
        }

        #[pymethod]
        fn appendleft(&self, item: PyObjectRef) {
            self.append_item(item, true, self.maxlen);
        }

        #[pymethod]
        fn clear(&self) {
            let removed = {
                let mut deque = self.borrow_deque_mut();
                if deque.is_empty() {
                    return;
                }
                self.state.fetch_add(1);
                core::mem::take(&mut *deque)
            };
            // Finalizers may read or repopulate the now-empty deque.
            drop(removed);
        }

        #[pymethod(name = "__copy__")]
        #[pymethod]
        fn copy(zelf: PyRef<PyDeque>, vm: &VirtualMachine) -> PyResult<PyRef<PyDeque>> {
            PyDeque {
                deque: PyRwLock::new(zelf.borrow_deque().clone()),
                maxlen: zelf.maxlen,
                state: AtomicCell::new(zelf.state.load()),
            }
            .into_ref_with_type(vm, zelf.class().to_owned())
        }

        #[pymethod]
        fn count(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
            let start_state = self.state.load();
            let count = self.mut_count(vm, &value)?;

            if start_state != self.state.load() {
                return Err(vm.new_runtime_error("deque mutated during iteration"));
            }
            Ok(count)
        }

        #[pymethod]
        fn extend(&self, iterable: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            self._extend(&iterable, false, vm)
        }

        #[pymethod]
        fn extendleft(&self, iterable: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            self._extend(&iterable, true, vm)
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
        fn insert(&self, index: i32, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            self.state.fetch_add(1);
            let mut deque = self.borrow_deque_mut();

            if self.maxlen == Some(deque.len()) {
                return Err(vm.new_index_error("deque already at its maximum size"));
            }

            let index = if index < 0 {
                if -index as usize > deque.len() {
                    0
                } else {
                    deque.len() - ((-index) as usize)
                }
            } else if index as usize > deque.len() {
                deque.len()
            } else {
                index as usize
            };

            deque.insert(index, value);

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
        fn remove(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            let start_state = self.state.load();
            let len = self.borrow_deque().len();
            let mutated = || vm.new_index_error("deque mutated during iteration");
            for index in 0..len {
                let item = self
                    .borrow_deque()
                    .get(index)
                    .cloned()
                    .ok_or_else(mutated)?;
                let equal = item.rich_compare_bool(&value, PyComparisonOp::Eq, vm)?;
                // Releasing the comparison reference can also run a finalizer.
                drop(item);
                if start_state != self.state.load() {
                    return Err(mutated());
                }
                if equal {
                    let removed = {
                        let mut deque = self.borrow_deque_mut();
                        if start_state != self.state.load() {
                            return Err(mutated());
                        }
                        let removed = deque.remove(index).ok_or_else(mutated)?;
                        self.state.fetch_add(1);
                        removed
                    };
                    drop(removed);
                    return Ok(());
                }
            }
            Err(vm.new_value_error("deque.remove(x): x not in deque"))
        }

        #[pymethod]
        fn reverse(&self) {
            let rev: VecDeque<_> = self.borrow_deque().iter().cloned().rev().collect();
            *self.borrow_deque_mut() = rev;
        }

        #[pymethod]
        fn __reversed__(zelf: PyRef<PyDeque>) -> PyReverseDequeIterator {
            PyReverseDequeIterator {
                state: zelf.state.load(),
                counter: AtomicCell::new(zelf.__len__()),
                internal: PyMutex::new(PositionIterInternal::new(zelf, 0)),
            }
        }

        #[pymethod]
        fn rotate(&self, args: RotateArgs) {
            self.state.fetch_add(1);
            let mut deque = self.borrow_deque_mut();
            if !deque.is_empty() {
                let n = args.n % deque.len() as isize;
                if n.is_negative() {
                    deque.rotate_left(-n as usize);
                } else {
                    deque.rotate_right(n as usize);
                }
            }
        }

        #[pygetset]
        fn maxlen(&self) -> Option<usize> {
            self.maxlen
        }

        #[pymethod]
        fn __reduce__(zelf: PyRef<PyDeque>, vm: &VirtualMachine) -> PyResult {
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
            object: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyGenericAlias> {
            PyGenericAlias::from_args(cls, object, vm)
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
            zelf: &Py<Self>,
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

            // SAFETY: This is hacky part for read-only field
            // Because `maxlen` is only mutated from __init__. We can abuse the lock of deque to ensure this is locked enough.
            // If we make a single lock of deque not only for extend but also for setting maxlen, it will be safe.
            let removed = {
                let mut deque = zelf.borrow_deque_mut();
                unsafe {
                    // `maxlen` is better to be defined as UnsafeCell in common practice,
                    // but then more type works without any safety benefits
                    let unsafe_maxlen =
                        &zelf.maxlen as *const _ as *const core::cell::UnsafeCell<Option<usize>>;
                    *(*unsafe_maxlen).get() = maxlen;
                }
                let removed = core::mem::take(&mut *deque);
                if !removed.is_empty() {
                    zelf.state.fetch_add(1);
                }
                removed
            };
            // Release old elements before iterating; their finalizers can add new ones.
            drop(removed);

            let Some(iterable) = iterable.into_option() else {
                return Ok(());
            };
            zelf._extend(&iterable, false, vm)
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
                    zelf._extend(other, false, vm)?;
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
            crate::iter::richcompare_mutating_seqs(
                |i| {
                    let lhs = zelf.borrow_deque();
                    (lhs.len(), lhs.get(i).cloned())
                },
                |i| {
                    let rhs = other.borrow_deque();
                    (rhs.len(), rhs.get(i).cloned())
                },
                op,
                vm,
            )
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
                    deque.iter().map(|o| &**o),
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
    #[pyclass(name = "_deque_iterator", module = "collections", traverse)]
    #[derive(Debug, PyPayload)]
    struct PyDequeIterator {
        #[pytraverse(skip)]
        state: usize,
        /// How many elements are left to walk, `dequeiterobject.counter`. Kept
        /// beside the deque rather than read back from it, because a mutated
        /// deque is walked no further and what is left of it then reads as
        /// nothing.
        #[pytraverse(skip)]
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

    impl PyDequeIterator {
        pub(crate) fn new(deque: PyDequeRef) -> Self {
            Self {
                state: deque.state.load(),
                counter: AtomicCell::new(deque.__len__()),
                internal: PyMutex::new(PositionIterInternal::new(deque, 0)),
            }
        }
    }

    #[pyclass(with(IterNext, Iterable, Constructor))]
    impl Py<PyDequeIterator> {
        #[pymethod]
        fn __length_hint__(&self) -> usize {
            self.counter.load()
        }

        #[pymethod]
        fn __reduce__(
            zelf: PyRef<PyDequeIterator>,
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
    #[pyclass(name = "_deque_reverse_iterator", module = "collections", traverse)]
    #[derive(Debug, PyPayload)]
    struct PyReverseDequeIterator {
        #[pytraverse(skip)]
        state: usize,
        /// As in [`PyDequeIterator`].
        #[pytraverse(skip)]
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
            let iter = Py::<PyDeque>::__reversed__(deque);
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
    impl Py<PyReverseDequeIterator> {
        #[pymethod]
        fn __length_hint__(&self) -> usize {
            self.counter.load()
        }

        #[pymethod]
        fn __reduce__(
            zelf: PyRef<PyReverseDequeIterator>,
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
    #[derive(Debug)]
    struct PyDefaultDict {
        dict: PyDict,
        #[pymember(writable)]
        default_factory: PyAtomicRef<Option<PyObject>>,
    }

    impl Default for PyDefaultDict {
        fn default() -> Self {
            Self {
                dict: PyDict::default(),
                default_factory: PyAtomicRef::new_empty(),
            }
        }
    }

    // SAFETY: Traverse visits each owned Python reference at most once.
    unsafe impl Traverse for PyDefaultDict {
        fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
            self.dict.traverse(tracer_fn);
            self.default_factory.traverse(tracer_fn);
        }

        fn clear(&mut self, out: &mut Vec<PyObjectRef>) {
            Traverse::clear(&mut self.dict, out);
            if let Some(factory) = self.default_factory.store(None) {
                out.push(factory);
            }
        }
    }

    #[pyclass(
        with(AsMapping, AsNumber, Constructor, Initializer, Representable),
        flags(BASETYPE, MAPPING, HAS_DICT)
    )]
    impl Py<PyDefaultDict> {
        #[pymethod]
        fn __missing__(&self, object: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            let factory = self
                .default_factory
                .load_owned()
                .filter(|factory| !vm.is_none(factory));

            if let Some(f) = factory {
                let value = f.call((), vm)?;
                self.dict.setdefault(object, value, vm)
            } else {
                Err(vm.new_key_error(object))
            }
        }

        #[pymethod]
        #[pymethod(name = "__copy__")]
        fn copy(&self) -> PyDefaultDict {
            let default_factory = match self.default_factory.load_owned() {
                Some(factory) => PyAtomicRef::from(Some(factory)),
                None => PyAtomicRef::new_empty(),
            };

            PyDefaultDict {
                dict: self.dict.copy(),
                default_factory,
            }
        }

        #[pymethod]
        fn __reduce__(zelf: PyRef<PyDefaultDict>, vm: &VirtualMachine) -> PyResult {
            let cls = zelf.class().to_owned();

            // NULL and None both mean "no factory": the args tuple is empty.
            let default_factory = zelf
                .default_factory
                .load_owned()
                .filter(|factory| !vm.is_none(factory));
            let factory_tuple_elements =
                default_factory.map_or_else(Vec::new, |factory| vec![factory]);
            let factory_tuple = vm.ctx.new_tuple(factory_tuple_elements);

            let items_fn = zelf.as_object().get_attr("items", vm)?;
            let items_iter = items_fn.call((), vm)?;
            let iter = PyIter::try_from_object(vm, items_iter)?;
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
        fn __or__(lhs: &PyObject, rhs: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            let not_implemented = || Ok(vm.ctx.not_implemented.clone().into());

            let (default_factory, dict) = if let Some(zelf) = lhs.downcast_ref::<Self>() {
                if !rhs.fast_isinstance(vm.ctx.types.dict_type) {
                    return not_implemented();
                }

                (zelf.default_factory.load_owned(), zelf.dict.copy())
            } else if let Some(zelf) = rhs.downcast_ref::<Self>() {
                let Some(dict) = lhs.downcast_ref::<PyDict>() else {
                    return not_implemented();
                };

                (zelf.default_factory.load_owned(), dict.copy())
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
                default_factory: match default_factory {
                    Some(factory) => PyAtomicRef::from(Some(factory)),
                    None => PyAtomicRef::new_empty(),
                },
            }
            .to_pyobject(vm))
        }
    }

    impl DefaultConstructor for PyDefaultDict {}

    impl Initializer for PyDefaultDict {
        type Args = FuncArgs;

        fn init(zelf: &Py<Self>, mut args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
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

            zelf.default_factory.store(default_factory);

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
            let default_factory = zelf.default_factory.load_owned();

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
                or: Some(|a, b, vm| PyDefaultDict::__or__(a, b.to_pyobject(vm), vm)),
                ..PyNumberMethods::NOT_IMPLEMENTED
            };
            &AS_NUMBER
        }
    }

    #[pyfunction]
    fn _count_elements(
        mapping: PyObjectRef,
        iterable: ArgIterable,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let iter = iterable.iter(vm)?;
        let one = vm.ctx.new_int(1);
        let get_name = vm.ctx.intern_str("get");
        let dict_type = vm.ctx.types.dict_type;
        let class = mapping.class();
        let inherited_from_dict = |name| {
            class
                .get_attr(name)
                .zip(dict_type.get_attr(name))
                .is_some_and(|(own, dict)| own.is(&dict))
        };
        // A dict whose type keeps `dict.get`/`dict.__setitem__` is updated in place,
        // hashing each key once and never consulting `__missing__`.
        if let Some(dict) = mapping.downcast_ref::<PyDict>()
            && inherited_from_dict(get_name)
            && inherited_from_dict(identifier!(vm, __setitem__))
        {
            let entries = dict._as_dict_inner();
            for key in iter {
                let key = key?;
                let hash = key.hash(vm)?;
                let count = match entries.get_known_hash(vm, &*key, hash)? {
                    Some(old) => vm._add(&old, one.as_object())?,
                    None => one.clone().into(),
                };
                entries.insert_known_hash(vm, &*key, hash, count)?;
            }
            return Ok(());
        }
        let get = mapping.get_attr(get_name, vm)?;
        let zero: PyObjectRef = vm.ctx.new_int(0).into();
        for key in iter {
            let key = key?;
            let old = get.call((key.clone(), zero.clone()), vm)?;
            let count = vm._add(&old, one.as_object())?;
            mapping.set_item(&*key, count, vm)?;
        }
        Ok(())
    }

    #[pyattr]
    #[pyclass(module = "collections", name = "_tuplegetter", traverse)]
    #[derive(Debug, PyPayload)]
    struct PyTupleGetter {
        #[pytraverse(skip)]
        index: isize,
        #[pymember(name = "__doc__", writable)]
        doc: PyAtomicRef<Option<PyObject>>,
    }

    impl Constructor for PyTupleGetter {
        type Args = (PySsize, PyObjectRef);

        fn py_new(
            _cls: &Py<PyType>,
            (index, doc): Self::Args,
            _vm: &VirtualMachine,
        ) -> PyResult<Self> {
            Ok(Self {
                index,
                doc: PyAtomicRef::from(Some(doc)),
            })
        }
    }

    #[pyclass(with(Constructor, GetDescriptor, Representable))]
    impl PyTupleGetter {
        fn doc(&self, vm: &VirtualMachine) -> PyObjectRef {
            self.doc.load_owned().unwrap_or_else(|| vm.ctx.none())
        }

        #[pyslot]
        fn descr_set(
            _zelf: &PyObject,
            _obj: PyObjectRef,
            value: PySetterValue,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            Err(vm.new_attribute_error(match value {
                PySetterValue::Assign(_) => "can't set attribute",
                PySetterValue::Delete => "can't delete attribute",
            }))
        }

        #[pymethod]
        fn __reduce__(zelf: PyRef<Self>, vm: &VirtualMachine) -> (PyTypeRef, (isize, PyObjectRef)) {
            (zelf.class().to_owned(), (zelf.index, zelf.doc(vm)))
        }
    }

    impl GetDescriptor for PyTupleGetter {
        fn descr_get(
            zelf: &PyObject,
            obj: Option<&PyObject>,
            _cls: Option<&PyObject>,
            vm: &VirtualMachine,
        ) -> PyResult {
            let (zelf, obj) = Self::_unwrap(zelf, obj, vm)?;
            if vm.is_none(obj) {
                return Ok(zelf.to_owned().into());
            }
            let Some(tuple) = obj.downcast_ref::<PyTuple>() else {
                return Err(vm.new_type_error(format!(
                    "descriptor for index '{}' for tuple subclasses doesn't apply to '{}' object",
                    zelf.index,
                    obj.class().name()
                )));
            };
            usize::try_from(zelf.index)
                .ok()
                .and_then(|index| tuple.as_slice().get(index))
                .cloned()
                .ok_or_else(|| vm.new_index_error("tuple index out of range"))
        }
    }

    impl Representable for PyTupleGetter {
        fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
            let doc = zelf.doc(vm).repr(vm)?;
            Ok(format!("{}({}, {})", zelf.class().name(), zelf.index, doc))
        }
    }
}
