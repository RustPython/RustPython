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
            PositionIterInternal, PyAnyDict, PyDict, PyGenericAlias, PyInt, PyList, PyStr, PyTuple,
            PyType, PyTypeRef,
        },
        common::lock::{
            PyMappedRwLockReadGuard, PyMappedRwLockWriteGuard, PyMutex, PyRwLock,
            PyRwLockReadGuard, PyRwLockWriteGuard,
        },
        convert::ToPyObject,
        function::{
            ArgIterable, FuncArgs, KwArgs, OptionalArg, PyComparisonValue, PySetterValue, PySsize,
        },
        object::{PyAtomicRef, Traverse, TraverseFn},
        protocol::{PyIter, PyIterReturn, PyMappingMethods, PyNumberMethods, PySequenceMethods},
        recursion::ReprGuard,
        sequence::MutObjectSequenceOp,
        sliceable::SequenceIndexOp,
        types::{
            AsMapping, AsNumber, AsSequence, Comparable, Constructor, DefaultConstructor,
            GetDescriptor, Initializer, IterNext, Iterable, PyComparisonOp, Representable,
            SelfIter,
        },
        utils::collection_repr,
        vm::{MAX_MEMORY_SIZE, thread::PyOperation},
    };
    use alloc::collections::VecDeque;
    use core::{cmp::max, mem::size_of};
    use crossbeam_utils::atomic::AtomicCell;
    use num_traits::{Signed, ToPrimitive};

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
    #[derive(Debug, PyPayload)]
    struct PyDeque {
        data: PyRwLock<DequeData>,
        operation: PyOperation,
        state: AtomicCell<usize>, // incremented whenever the indices move
    }

    #[derive(Debug, Default)]
    struct DequeData {
        items: VecDeque<PyObjectRef>,
        maxlen: Option<usize>,
    }

    // SAFETY: Traverse visits each owned Python reference at most once.
    unsafe impl Traverse for PyDeque {
        fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
            if let Some(deque) = self.data.try_read_recursive() {
                for obj in &deque.items {
                    obj.traverse(tracer_fn);
                }
            }
        }

        fn clear(&mut self, out: &mut Vec<PyObjectRef>) {
            out.extend(self.data.get_mut().items.drain(..));
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

    #[derive(FromArgs)]
    struct DequeIndexArgs {
        #[pyarg(positional, default = 0)]
        start: PyObjectRef,
        #[pyarg(positional, optional)]
        stop: OptionalArg<PyObjectRef>,
    }

    fn deque_index_bound(obj: &PyObject, vm: &VirtualMachine) -> PyResult<isize> {
        let index = obj.try_index_opt(vm).unwrap_or_else(|| {
            Err(vm.new_type_error("slice indices must be integers or have an __index__ method"))
        })?;
        let value = index.as_bigint();
        Ok(value.to_isize().unwrap_or_else(|| {
            if value.is_negative() {
                isize::MIN
            } else {
                isize::MAX
            }
        }))
    }

    impl PyDeque {
        fn new(vm: &VirtualMachine) -> Self {
            Self::from_items(VecDeque::new(), None, 0, vm)
        }

        fn from_items(
            items: VecDeque<PyObjectRef>,
            maxlen: Option<usize>,
            state: usize,
            vm: &VirtualMachine,
        ) -> Self {
            Self {
                data: PyRwLock::new(DequeData { items, maxlen }),
                operation: PyOperation::new(vm),
                state: AtomicCell::new(state),
            }
        }

        fn borrow_deque(&self) -> PyMappedRwLockReadGuard<'_, VecDeque<PyObjectRef>> {
            PyRwLockReadGuard::map(self.data.read(), |data| &data.items)
        }

        fn borrow_deque_mut(&self) -> PyMappedRwLockWriteGuard<'_, VecDeque<PyObjectRef>> {
            PyRwLockWriteGuard::map(self.data.write(), |data| &mut data.items)
        }

        fn maxlen_value(&self) -> Option<usize> {
            self.data.read().maxlen
        }

        fn snapshot(&self) -> (VecDeque<PyObjectRef>, Option<usize>, usize) {
            let data = self.data.read();
            (data.items.clone(), data.maxlen, self.state.load())
        }

        fn append_item(&self, item: PyObjectRef, left: bool, maxlen: Option<usize>) {
            let removed = {
                let mut deque = self.borrow_deque_mut();
                self.push_item(&mut deque, item, left, maxlen)
            };
            drop(removed);
        }

        // Return evicted items to the caller so they can be dropped after unlocking.
        fn push_item(
            &self,
            deque: &mut VecDeque<PyObjectRef>,
            item: PyObjectRef,
            left: bool,
            maxlen: Option<usize>,
        ) -> Option<PyObjectRef> {
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
            self.operation
                .with(vm, || self.extend_inner(iterable, left, vm))
        }

        fn extend_inner(
            &self,
            iterable: &PyObject,
            left: bool,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            if iterable
                .downcast_ref::<Self>()
                .is_some_and(|other| core::ptr::eq(self, &**other))
            {
                // Snapshot self-extension through the iterator protocol so
                // subclass overrides of __iter__ are still respected.
                let elements: Vec<PyObjectRef> = iterable.try_to_value(vm)?;
                // Retain the snapshot until extension finishes, as a temporary list does.
                return self.extend_items(
                    elements.iter().cloned().map(Ok),
                    left,
                    self.maxlen_value(),
                );
            }

            let maxlen = self.maxlen_value();
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
            let item = {
                let deque = self.borrow_deque();
                idx.wrapped_at(deque.len())
                    .and_then(|i| deque.get(i).cloned())
            };
            item.ok_or_else(|| vm.new_index_error("deque index out of range"))
        }

        fn __setitem__(&self, idx: isize, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            self.operation.with(vm, || {
                let mut value = Some(value);
                let removed = {
                    let mut deque = self.borrow_deque_mut();
                    idx.wrapped_at(deque.len())
                        .map(|i| core::mem::replace(&mut deque[i], value.take().unwrap()))
                };
                let removed =
                    removed.ok_or_else(|| vm.new_index_error("deque index out of range"))?;
                drop(removed);
                Ok(())
            })
        }

        fn __delitem__(&self, idx: isize, vm: &VirtualMachine) -> PyResult<()> {
            self.operation.with(vm, || {
                let removed = {
                    let mut deque = self.borrow_deque_mut();
                    idx.wrapped_at(deque.len()).and_then(|i| {
                        let removed = deque.remove(i);
                        self.state.fetch_add(1);
                        removed
                    })
                };
                let removed =
                    removed.ok_or_else(|| vm.new_index_error("deque index out of range"))?;
                drop(removed);
                Ok(())
            })
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

        fn _mul(
            &self,
            n: isize,
            vm: &VirtualMachine,
        ) -> PyResult<(VecDeque<PyObjectRef>, Option<usize>)> {
            let (deque, maxlen, _) = self.snapshot();
            let n = vm.check_repeat_or_overflow_error(deque.len(), n)?;
            let mul_len = n * deque.len();
            let result_len = maxlen.map_or(mul_len, |maxlen| mul_len.min(maxlen));
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
            Ok((result, maxlen))
        }

        fn __mul__(zelf: &Py<Self>, n: isize, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
            zelf.operation.with(vm, || {
                if !zelf.class().is(Self::class(&vm.ctx)) {
                    return Self::__imul__(Py::<Self>::copy(zelf.to_owned(), vm)?, n, vm);
                }
                let (items, maxlen) = zelf._mul(n, vm)?;
                Ok(Self::from_items(items, maxlen, 0, vm).into_ref(&vm.ctx))
            })
        }

        fn __imul__(zelf: PyRef<Self>, n: isize, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
            zelf.operation.with(vm, || Self::imul_inner(&zelf, n, vm))?;
            Ok(zelf)
        }

        fn imul_inner(zelf: &Py<Self>, n: isize, vm: &VirtualMachine) -> PyResult<()> {
            if n == 1 || zelf.borrow_deque().is_empty() {
                return Ok(());
            }
            let size = zelf.__len__();
            if n > 1 && size > 1 && !zelf.class().is(Self::class(&vm.ctx)) {
                let mut repeats = n as usize;
                if size > isize::MAX as usize / repeats {
                    return Err(vm.no_memory_error());
                }
                // Snapshot through the iterator protocol without a deque lock. Retain
                // the snapshot until all extensions finish, including item finalizers.
                let items = vm.extract_elements_sized(zelf.as_object(), &|| 0, Ok)?;
                if let Some(maxlen) = zelf.maxlen_value() {
                    repeats = repeats.min(maxlen.div_ceil(size));
                }
                if items.is_empty() || repeats <= 1 {
                    return Ok(());
                }
                if zelf.maxlen_value().is_none() {
                    // Use the snapshot length: subclass callbacks may change both
                    // the deque and the items being repeated. Reservation also
                    // rejects capacities whose byte size cannot be represented.
                    let additional = items
                        .len()
                        .checked_mul(repeats - 1)
                        .ok_or_else(|| vm.no_memory_error())?;
                    let reserved = zelf.borrow_deque_mut().try_reserve_exact(additional);
                    reserved.map_err(|_| vm.no_memory_error())?;
                }
                for _ in 1..repeats {
                    let maxlen = zelf.maxlen_value();
                    if maxlen == Some(0) {
                        continue;
                    }
                    for item in &items {
                        let removed = {
                            let mut deque = zelf.borrow_deque_mut();
                            // Finalizers or another thread can discard a previous
                            // reservation, so check capacity under the push lock.
                            deque
                                .try_reserve(1)
                                .map(|()| zelf.push_item(&mut deque, item.clone(), false, maxlen))
                        };
                        let removed = removed.map_err(|_| vm.no_memory_error())?;
                        drop(removed);
                    }
                }
                return Ok(());
            }
            let (mut mul_deque, _) = zelf._mul(n, vm)?;
            {
                let mut deque = zelf.borrow_deque_mut();
                core::mem::swap(&mut *deque, &mut mul_deque);
                zelf.state.fetch_add(1);
            }
            drop(mul_deque);
            Ok(())
        }

        fn __len__(&self) -> usize {
            self.borrow_deque().len()
        }

        fn concat(zelf: &Py<Self>, other: &PyObject, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
            zelf.operation.with(vm, || {
                if let Some(o) = other.downcast_ref::<Self>() {
                    if !zelf.class().is(Self::class(&vm.ctx))
                        || !other.class().is(Self::class(&vm.ctx))
                    {
                        let result = Py::<Self>::copy(zelf.to_owned(), vm)?;
                        result._extend(other, false, vm)?;
                        return Ok(result);
                    }
                    let (mut deque, maxlen, _) = zelf.snapshot();
                    let elements = o.borrow_deque().clone();
                    deque.extend(elements);

                    let skipped = maxlen
                        .and_then(|maxlen| deque.len().checked_sub(maxlen))
                        .unwrap_or(0);
                    deque.drain(..skipped);

                    Ok(Self::from_items(deque, maxlen, 0, vm).into_ref(&vm.ctx))
                } else {
                    Err(vm.new_type_error(format!(
                        r#"can only concatenate deque (not "{}") to deque"#,
                        other.class().name()
                    )))
                }
            })
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
        fn append(&self, item: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            self.operation.with(vm, || {
                self.append_item(item, false, self.maxlen_value());
                Ok(())
            })
        }

        #[pymethod]
        fn appendleft(&self, item: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            self.operation.with(vm, || {
                self.append_item(item, true, self.maxlen_value());
                Ok(())
            })
        }

        #[pymethod]
        fn clear(&self, vm: &VirtualMachine) -> PyResult<()> {
            self.operation.with(vm, || {
                let removed = {
                    let mut deque = self.borrow_deque_mut();
                    if deque.is_empty() {
                        return Ok(());
                    }
                    self.state.fetch_add(1);
                    core::mem::take(&mut *deque)
                };
                // Finalizers may read or repopulate the now-empty deque.
                drop(removed);
                Ok(())
            })
        }

        #[pymethod(name = "__copy__")]
        #[pymethod]
        fn copy(zelf: PyRef<PyDeque>, vm: &VirtualMachine) -> PyResult<PyRef<PyDeque>> {
            zelf.operation.with(vm, || {
                let cls = zelf.class();
                if !cls.is(PyDeque::class(&vm.ctx)) {
                    // A subclass constructor may change the contents or maxlen. Do not hold
                    // the deque lock across Python callbacks, and validate its result.
                    let result = match zelf.maxlen_value() {
                        Some(maxlen) => cls.as_object().call((zelf.clone(), maxlen), vm)?,
                        None => cls.as_object().call((zelf.clone(),), vm)?,
                    };
                    return result.downcast().map_err(|obj| {
                        vm.new_type_error(format!(
                            "{}() must return a deque, not {}",
                            cls.name(),
                            obj.class().name()
                        ))
                    });
                }
                let (items, maxlen, state) = zelf.snapshot();
                PyDeque::from_items(items, maxlen, state, vm)
                    .into_ref_with_type(vm, zelf.class().to_owned())
            })
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
            range: DequeIndexArgs,
            vm: &VirtualMachine,
        ) -> PyResult<usize> {
            // Argument conversion runs before the deque's critical section.
            let start = deque_index_bound(&range.start, vm)?;
            let stop = match range.stop {
                OptionalArg::Present(stop) => deque_index_bound(&stop, vm)?,
                OptionalArg::Missing => isize::MAX,
            };
            self.operation.with(vm, || {
                let (start, stop, state) = {
                    let data = self.data.read();
                    (
                        start.saturated_at(data.items.len()),
                        stop.saturated_at(data.items.len()),
                        self.state.load(),
                    )
                };
                for index in start..stop {
                    let item = self.borrow_deque().get(index).cloned();
                    let item =
                        item.ok_or_else(|| vm.new_runtime_error("deque mutated during iteration"))?;
                    let equal = item.rich_compare_bool(&needle, PyComparisonOp::Eq, vm);
                    drop(item);
                    if equal? {
                        return Ok(index);
                    }
                    if state != self.state.load() {
                        return Err(vm.new_runtime_error("deque mutated during iteration"));
                    }
                }
                Err(vm.new_value_error("deque.index(x): x not in deque"))
            })
        }

        #[pymethod]
        fn insert(&self, index: i32, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            self.operation.with(vm, || {
                let mut value = Some(value);
                let full = {
                    let mut data = self.data.write();
                    if data.maxlen == Some(data.items.len()) {
                        true
                    } else {
                        let len = data.items.len();
                        let index = if index < 0 {
                            len.saturating_sub(index.unsigned_abs() as usize)
                        } else {
                            (index as usize).min(len)
                        };
                        data.items.insert(index, value.take().unwrap());
                        self.state.fetch_add(1);
                        false
                    }
                };
                if full {
                    Err(vm.new_index_error("deque already at its maximum size"))
                } else {
                    Ok(())
                }
            })
        }

        #[pymethod]
        fn pop(&self, vm: &VirtualMachine) -> PyResult {
            self.operation.with(vm, || {
                let item = {
                    let mut deque = self.borrow_deque_mut();
                    let item = deque.pop_back();
                    if item.is_some() {
                        self.state.fetch_add(1);
                    }
                    item
                };
                item.ok_or_else(|| vm.new_index_error("pop from an empty deque"))
            })
        }

        #[pymethod]
        fn popleft(&self, vm: &VirtualMachine) -> PyResult {
            self.operation.with(vm, || {
                let item = {
                    let mut deque = self.borrow_deque_mut();
                    let item = deque.pop_front();
                    if item.is_some() {
                        self.state.fetch_add(1);
                    }
                    item
                };
                item.ok_or_else(|| vm.new_index_error("pop from an empty deque"))
            })
        }

        #[pymethod]
        fn remove(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            self.operation.with(vm, || {
                let (len, state) = {
                    let data = self.data.read();
                    (data.items.len(), self.state.load())
                };
                for index in 0..len {
                    let item = self.borrow_deque().get(index).cloned();
                    let item =
                        item.ok_or_else(|| vm.new_index_error("deque mutated during iteration"))?;
                    let equal = item.rich_compare_bool(&value, PyComparisonOp::Eq, vm);
                    // Releasing the comparison reference can also run a finalizer.
                    drop(item);
                    let equal = equal?;
                    if state != self.state.load() {
                        return Err(vm.new_index_error("deque mutated during iteration"));
                    }
                    if equal {
                        let removed = {
                            let mut deque = self.borrow_deque_mut();
                            if state != self.state.load() {
                                None
                            } else {
                                let removed = deque.remove(index);
                                if removed.is_some() {
                                    self.state.fetch_add(1);
                                }
                                removed
                            }
                        };
                        let removed = removed
                            .ok_or_else(|| vm.new_index_error("deque mutated during iteration"))?;
                        drop(removed);
                        return Ok(());
                    }
                }
                Err(vm.new_value_error("deque.remove(x): x not in deque"))
            })
        }

        #[pymethod]
        fn reverse(&self, vm: &VirtualMachine) -> PyResult<()> {
            self.operation.with(vm, || {
                self.borrow_deque_mut().make_contiguous().reverse();
                Ok(())
            })
        }

        #[pymethod]
        fn __reversed__(zelf: PyRef<PyDeque>) -> PyReverseDequeIterator {
            let (state, len) = {
                let data = zelf.data.read();
                (zelf.state.load(), data.items.len())
            };
            PyReverseDequeIterator {
                state,
                counter: AtomicCell::new(len),
                internal: PyMutex::new(PositionIterInternal::new(zelf, 0)),
            }
        }

        #[pymethod]
        fn rotate(&self, args: RotateArgs, vm: &VirtualMachine) -> PyResult<()> {
            self.operation.with(vm, || {
                let mut deque = self.borrow_deque_mut();
                if deque.len() > 1 {
                    self.state.fetch_add(1);
                    let n = args.n % deque.len() as isize;
                    if n.is_negative() {
                        deque.rotate_left(n.unsigned_abs());
                    } else {
                        deque.rotate_right(n as usize);
                    }
                }
                Ok(())
            })
        }

        #[pygetset]
        fn maxlen(&self) -> Option<usize> {
            self.maxlen_value()
        }

        #[pymethod]
        fn __reduce__(zelf: PyRef<PyDeque>, vm: &VirtualMachine) -> PyResult {
            // Use __getstate__ to capture both __dict__ and __slots__ values so
            // subclass attributes survive a pickle round-trip (matches CPython's
            // deque___reduce___impl, which calls _PyObject_GetState).
            let state = vm.call_method(zelf.as_object(), "__getstate__", ())?;
            let iterator = zelf.as_object().get_iter(vm)?;
            let cls = zelf.class().to_owned();
            let value = match zelf.maxlen_value() {
                Some(v) => vm.new_pyobj((vm.ctx.empty_tuple.clone(), v)),
                None => vm.ctx.empty_tuple.clone().into(),
            };
            Ok(vm.new_pyobj((cls, value, state, iterator)))
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

    impl Constructor for PyDeque {
        type Args = FuncArgs;

        fn slot_new(cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            Self::new(vm).into_ref_with_type(vm, cls).map(Into::into)
        }

        fn py_new(_cls: &Py<PyType>, _args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            Ok(Self::new(vm))
        }
    }

    impl Initializer for PyDeque {
        type Args = PyDequeOptions;

        fn init(
            zelf: &Py<Self>,
            PyDequeOptions { iterable, maxlen }: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            zelf.operation.with(vm, || {
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

                let removed = {
                    let mut data = zelf.data.write();
                    data.maxlen = maxlen;
                    let removed = core::mem::take(&mut data.items);
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
                zelf.extend_inner(&iterable, false, vm)
            })
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
                    PyDeque::concat(PyDeque::sequence_downcast(seq), other, vm).map(Into::into)
                }),

                repeat: atomic_func!(|seq, n, vm| {
                    PyDeque::__mul__(PyDeque::sequence_downcast(seq), n, vm).map(Into::into)
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
            let (deque, maxlen, _) = zelf.snapshot();
            let class = zelf.class();
            let class_name = class.name();
            let closing_part =
                maxlen.map_or_else(|| "]".to_owned(), |maxlen| format!("], maxlen={maxlen}"));
            let empty = format!("{class_name}([{closing_part})");

            if deque.is_empty() {
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
            let (state, len) = {
                let data = deque.data.read();
                (deque.state.load(), data.items.len())
            };
            Self {
                state,
                counter: AtomicCell::new(len),
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
            let (deque, position) = {
                let internal = zelf.internal.lock();
                let deque = match &internal.status {
                    Active(obj) => Some(obj.clone()),
                    Exhausted => None,
                };
                (deque, internal.position)
            };
            let deque = deque.unwrap_or_else(|| PyDeque::new(vm).into_ref(&vm.ctx));
            (
                zelf.class().to_owned(),
                (deque, vm.ctx.new_int(position).into()),
            )
        }
    }

    impl SelfIter for PyDequeIterator {}

    enum DequeStep {
        Item(PyObjectRef),
        Exhausted,
        Mutated,
    }

    impl DequeStep {
        fn into_result(self, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            match self {
                Self::Item(item) => Ok(PyIterReturn::Return(item)),
                Self::Exhausted => Ok(PyIterReturn::StopIteration(None)),
                Self::Mutated => Err(vm.new_runtime_error("deque mutated during iteration")),
            }
        }
    }

    fn deque_step(
        internal: &PyMutex<PositionIterInternal<PyDequeRef>>,
        state: usize,
        counter: &AtomicCell<usize>,
        reverse: bool,
    ) -> (DequeStep, Option<PyDequeRef>) {
        let mut guard = internal.lock();
        let internal = &mut *guard;
        // The count before the deque, as in `dequereviter_next()`, so
        // an iterator that has raised once runs out instead.
        if reverse && counter.load() == 0 {
            return (DequeStep::Exhausted, internal.exhaust());
        }
        let item = match &internal.status {
            Active(deque) => {
                let data = deque.data.read();
                // The deque before the count, as in `deque_iternext()`, so an
                // iterator still holding a deque that moved raises again on
                // every call rather than running out after the first.
                if state != deque.state.load() {
                    counter.store(0);
                    return (DequeStep::Mutated, None);
                }
                if counter.load() == 0 {
                    None
                } else {
                    let index = if reverse {
                        data.items.len().checked_sub(internal.position + 1)
                    } else {
                        Some(internal.position)
                    };
                    let item = index.and_then(|index| data.items.get(index).cloned());
                    if item.is_some() {
                        internal.position += 1;
                        counter.store(counter.load() - 1);
                    } else {
                        counter.store(0);
                    }
                    item
                }
            }
            Exhausted => None,
        };
        if let Some(item) = item {
            (DequeStep::Item(item), None)
        } else {
            counter.store(0);
            (DequeStep::Exhausted, internal.exhaust())
        }
    }

    impl IterNext for PyDequeIterator {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            let (step, released) = deque_step(&zelf.internal, zelf.state, &zelf.counter, false);
            drop(released);
            step.into_result(vm)
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
            let (deque, position) = {
                let internal = zelf.internal.lock();
                let deque = match &internal.status {
                    Active(obj) => Some(obj.clone()),
                    Exhausted => None,
                };
                (deque, internal.position)
            };
            let deque = deque.unwrap_or_else(|| PyDeque::new(vm).into_ref(&vm.ctx));
            (
                zelf.class().to_owned(),
                (deque, vm.ctx.new_int(position).into()),
            )
        }
    }

    impl SelfIter for PyReverseDequeIterator {}

    impl IterNext for PyReverseDequeIterator {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            let (step, released) = deque_step(&zelf.internal, zelf.state, &zelf.counter, true);
            drop(released);
            step.into_result(vm)
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

            let (zelf, other) = if let Some(zelf) = lhs.downcast_ref::<Self>() {
                (zelf, &*rhs)
            } else if let Some(zelf) = rhs.downcast_ref::<Self>() {
                (zelf, lhs)
            } else {
                return Err(vm.new_type_error(format!(
                    "unsupported operand type(s) for |: '{}' and '{}'",
                    lhs.class().name(),
                    rhs.class().name()
                )));
            };
            if PyAnyDict::from_object(other).is_none() {
                return not_implemented();
            }

            let factory = zelf
                .default_factory
                .load_owned()
                .unwrap_or_else(|| vm.ctx.none());
            let new = zelf
                .class()
                .as_object()
                .call((factory, lhs.to_owned()), vm)?;
            let dict = new
                .downcast_ref::<PyDict>()
                .ok_or_else(|| vm.new_system_error("bad argument to internal function"))?;
            dict.merge_object(rhs, vm)?;
            Ok(new)
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
                let count = match entries.get(vm, &*key, hash)? {
                    Some(old) => vm._add(&old, one.as_object())?,
                    None => one.clone().into(),
                };
                entries.insert(vm, &*key, hash, count)?;
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
