pub(crate) use _collections::make_module;

#[pymodule]
mod _collections {
    use crate::{
        AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
        atomic_func,
        builtins::{
            IterStatus::{Active, Exhausted},
            PositionIterInternal, PyDict, PyGenericAlias, PyInt, PyStrRef, PyType, PyTypeRef,
        },
        common::{
            ascii,
            lock::{PyMutex, PyRwLock, PyRwLockReadGuard, PyRwLockWriteGuard},
        },
        convert::ToPyObject,
        dict_inner,
        function::{ArgIterable, KwArgs, OptionalArg, PyComparisonValue},
        iter::PyExactSizeIterator,
        protocol::{PyIterReturn, PyMappingMethods, PyNumberMethods, PySequenceMethods},
        recursion::ReprGuard,
        sequence::{MutObjectSequenceOp, OptionalRangeArgs},
        sliceable::SequenceIndexOp,
        types::{
            AsMapping, AsNumber, AsSequence, Callable, Comparable, Constructor, DefaultConstructor,
            Initializer, IterNext, Iterable, PyComparisonOp, Representable, SelfIter,
        },
        utils::collection_repr,
    };
    use alloc::collections::VecDeque;
    use core::cmp::max;
    use crossbeam_utils::atomic::AtomicCell;

    #[pyattr]
    #[pyclass(module = "collections", name = "deque", unhashable = true)]
    #[derive(Debug, Default, PyPayload)]
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

    #[pyclass(
        flags(BASETYPE),
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
                        .map(|repr| format!("{repr} is not in deque"))
                        .unwrap_or_else(|_| String::new()),
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
        fn __reversed__(zelf: PyRef<Self>) -> PyResult<PyReverseDequeIterator> {
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
                    "can only concatenate deque (not \"{}\") to deque",
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
            Ok(vm.new_pyobj((cls, value, vm.ctx.none(), PyDequeIterator::new(zelf))))
        }

        #[pyclassmethod]
        fn __class_getitem__(
            cls: PyTypeRef,
            args: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyGenericAlias {
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
        fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
            let deque = zelf.borrow_deque().clone();
            let class = zelf.class();
            let class_name = class.name();
            let closing_part = zelf
                .maxlen
                .map(|maxlen| format!("], maxlen={maxlen}"))
                .unwrap_or_else(|| "]".to_owned());

            let s = if zelf.__len__() == 0 {
                format!("{class_name}([{closing_part})")
            } else if let Some(_guard) = ReprGuard::enter(vm, zelf.as_object()) {
                collection_repr(Some(&class_name), "[", &closing_part, deque.iter(), vm)?
            } else {
                "[...]".to_owned()
            };

            Ok(s)
        }
    }

    #[pyattr]
    #[pyclass(name = "_deque_iterator")]
    #[derive(Debug, PyPayload)]
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
            }
            Ok(iter)
        }
    }

    #[pyclass(with(IterNext, Iterable, Constructor))]
    impl PyDequeIterator {
        pub(crate) fn new(deque: PyDequeRef) -> Self {
            Self {
                state: deque.state.load(),
                internal: PyMutex::new(PositionIterInternal::new(deque, 0)),
            }
        }

        #[pymethod]
        fn __length_hint__(&self) -> usize {
            self.internal.lock().length_hint(|obj| obj.__len__())
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
    impl IterNext for PyDequeIterator {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            zelf.internal.lock().next(|deque, pos| {
                if zelf.state != deque.state.load() {
                    return Err(vm.new_runtime_error("Deque mutated during iteration"));
                }
                let deque = deque.borrow_deque();
                Ok(PyIterReturn::from_result(
                    deque.get(pos).cloned().ok_or(None),
                ))
            })
        }
    }

    #[pyattr]
    #[pyclass(name = "_deque_reverse_iterator")]
    #[derive(Debug, PyPayload)]
    struct PyReverseDequeIterator {
        state: usize,
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
            let iter = PyDeque::__reversed__(deque)?;
            if let OptionalArg::Present(index) = index {
                let index = max(index, 0) as usize;
                iter.internal.lock().position = index;
            }
            Ok(iter)
        }
    }

    #[pyclass(with(IterNext, Iterable, Constructor))]
    impl PyReverseDequeIterator {
        #[pymethod]
        fn __length_hint__(&self) -> usize {
            self.internal.lock().length_hint(|obj| obj.__len__())
        }

        #[pymethod]
        fn __reduce__(
            zelf: PyRef<Self>,
            vm: &VirtualMachine,
        ) -> PyResult<(PyTypeRef, (PyDequeRef, PyObjectRef))> {
            let internal = zelf.internal.lock();
            let deque = match &internal.status {
                Active(obj) => obj.clone(),
                Exhausted => PyDeque::default().into_ref(&vm.ctx),
            };
            Ok((
                zelf.class().to_owned(),
                (deque, vm.ctx.new_int(internal.position).into()),
            ))
        }
    }

    impl SelfIter for PyReverseDequeIterator {}
    impl IterNext for PyReverseDequeIterator {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            zelf.internal.lock().next(|deque, pos| {
                if deque.state.load() != zelf.state {
                    return Err(vm.new_runtime_error("Deque mutated during iteration"));
                }
                let deque = deque.borrow_deque();
                let r = deque
                    .len()
                    .checked_sub(pos + 1)
                    .and_then(|pos| deque.get(pos))
                    .cloned();
                Ok(PyIterReturn::from_result(r.ok_or(None)))
            })
        }
    }

    // ============================================================================
    // OrderedDict implementation
    // ============================================================================

    #[pyattr]
    #[pyclass(module = "_collections", name = "OrderedDict", base = PyDict, unhashable = true)]
    #[derive(Debug, Default)]
    pub struct PyOrderedDict {
        inner: PyDict,
    }

    pub type PyOrderedDictRef = PyRef<PyOrderedDict>;

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
            AsNumber,
            Representable
        )
    )]
    impl PyOrderedDict {
        /// Move an existing element to the end (or beginning if last is false).
        #[pymethod]
        fn move_to_end(&self, args: MoveToEndArgs, vm: &VirtualMachine) -> PyResult<()> {
            let entries = self.inner._as_dict_inner();
            match entries.move_to_end(vm, &*args.key, args.last)? {
                true => Ok(()),
                false => Err(vm.new_key_error(args.key)),
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
                            .map(|s| s.as_str().to_owned())
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
                Ok(pyodict) => {
                    for key in args.iterable.iter(vm)? {
                        let key: PyObjectRef = key?;
                        pyodict
                            .inner
                            ._as_dict_inner()
                            .insert(vm, &*key, value.clone())?;
                    }
                    Ok(pyodict.into_pyref().into())
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
        fn __len__(&self) -> usize {
            self.inner._as_dict_inner().len()
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

        #[pymethod]
        fn __contains__(&self, key: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
            self.inner._as_dict_inner().contains(vm, &*key)
        }

        #[pymethod]
        fn __getitem__(&self, key: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            match self.inner._as_dict_inner().get(vm, &*key)? {
                Some(value) => Ok(value),
                None => Err(vm.new_key_error(key)),
            }
        }

        #[pymethod]
        fn __setitem__(
            &self,
            key: PyObjectRef,
            value: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            self.inner._as_dict_inner().insert(vm, &*key, value)
        }

        #[pymethod]
        fn __delitem__(&self, key: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            self.inner._as_dict_inner().delete(vm, &*key)
        }

        /// Return a reverse iterator over the dict keys.
        #[pymethod]
        fn __reversed__(zelf: PyRef<Self>) -> PyOrderedDictReverseKeyIterator {
            PyOrderedDictReverseKeyIterator::new(zelf)
        }

        /// Return odict_keys view
        #[pymethod]
        fn keys(zelf: PyRef<Self>) -> PyOrderedDictKeys {
            PyOrderedDictKeys { odict: zelf }
        }

        /// Return odict_values view
        #[pymethod]
        fn values(zelf: PyRef<Self>) -> PyOrderedDictValues {
            PyOrderedDictValues { odict: zelf }
        }

        /// Return odict_items view
        #[pymethod]
        fn items(zelf: PyRef<Self>) -> PyOrderedDictItems {
            PyOrderedDictItems { odict: zelf }
        }

        #[pymethod]
        fn __reduce__(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
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
                .map(|d| d.into())
                .unwrap_or_else(|| vm.ctx.none());

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
                            .map(|s| s.as_str().to_owned())
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

            Ok(vm
                .new_tuple((zelf.class().to_owned(), vm.new_tuple((items_list,)), state))
                .into())
        }

        #[pyclassmethod]
        fn __class_getitem__(
            cls: PyTypeRef,
            args: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyGenericAlias {
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
            if let Some(other_odict) = other.downcast_ref::<PyOrderedDict>()
                && (op == PyComparisonOp::Eq || op == PyComparisonOp::Ne)
            {
                let self_items = zelf.inner._as_dict_inner().items();
                let other_items = other_odict.inner._as_dict_inner().items();

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

    impl AsNumber for PyOrderedDict {
        fn as_number() -> &'static PyNumberMethods {
            static AS_NUMBER: PyNumberMethods = PyNumberMethods {
                // Handle both __or__ and __ror__ in the same function
                // This function is used for both `or` and `right_or` slots via copy_from
                or: Some(|a, b, vm| {
                    let a_is_odict = a.downcast_ref::<PyOrderedDict>().is_some();
                    let b_is_odict = b.downcast_ref::<PyOrderedDict>().is_some();
                    let a_is_dict = a.class().fast_issubclass(vm.ctx.types.dict_type);
                    let b_is_dict = b.class().fast_issubclass(vm.ctx.types.dict_type);

                    if a_is_odict {
                        // This is __or__: OrderedDict | other
                        // other must be a dict or dict subclass
                        if !b_is_dict {
                            return Ok(vm.ctx.not_implemented());
                        }
                        let a_odict = a.downcast_ref::<PyOrderedDict>().unwrap();
                        let new_inner = a_odict.inner.copy();
                        new_inner.merge_object(b.to_pyobject(vm), vm)?;
                        // Preserve the subclass type (use a's type)
                        let result = PyOrderedDict { inner: new_inner }
                            .into_ref_with_type(vm, a.class().to_owned())?;
                        Ok(result.into())
                    } else if b_is_odict {
                        // This is __ror__: other | OrderedDict
                        // other must be a dict or dict subclass
                        if !a_is_dict {
                            return Ok(vm.ctx.not_implemented());
                        }
                        let b_odict = b.downcast_ref::<PyOrderedDict>().unwrap();
                        // Create new instance with b's type (preserve subclass)
                        let new_inner = PyDict::default();
                        new_inner.merge_object(a.to_pyobject(vm), vm)?;
                        for (key, value) in b_odict.inner._as_dict_inner().items() {
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
        odict: PyOrderedDictRef,
    }

    #[pyclass(with(Iterable, Comparable, Representable))]
    impl PyOrderedDictKeys {
        #[pymethod]
        fn __len__(&self) -> usize {
            self.odict.inner._as_dict_inner().len()
        }

        #[pymethod]
        fn __contains__(&self, key: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
            self.odict.inner._as_dict_inner().contains(vm, &*key)
        }

        #[pymethod]
        fn __reversed__(&self) -> PyOrderedDictReverseKeyIterator {
            PyOrderedDictReverseKeyIterator::new(self.odict.clone())
        }
    }

    impl Iterable for PyOrderedDictKeys {
        fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            Ok(PyOrderedDictKeyIterator::new(zelf.odict.clone()).into_pyobject(vm))
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
            let self_keys: Vec<PyObjectRef> = zelf.odict.inner._as_dict_inner().keys();
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
                let mut str_parts = Vec::with_capacity(zelf.odict.inner._as_dict_inner().len());
                for key in zelf.odict.inner._as_dict_inner().keys() {
                    let repr: PyStrRef = key.repr(vm)?;
                    str_parts.push(repr.as_str().to_owned());
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
        odict: PyOrderedDictRef,
    }

    #[pyclass(with(Iterable, Representable))]
    impl PyOrderedDictValues {
        #[pymethod]
        fn __len__(&self) -> usize {
            self.odict.inner._as_dict_inner().len()
        }

        #[pymethod]
        fn __reversed__(&self) -> PyOrderedDictReverseValueIterator {
            PyOrderedDictReverseValueIterator::new(self.odict.clone())
        }
    }

    impl Iterable for PyOrderedDictValues {
        fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            Ok(PyOrderedDictValueIterator::new(zelf.odict.clone()).into_pyobject(vm))
        }
    }

    impl Representable for PyOrderedDictValues {
        #[inline]
        fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
            if let Some(_guard) = ReprGuard::enter(vm, zelf.as_object()) {
                let mut str_parts = Vec::with_capacity(zelf.odict.inner._as_dict_inner().len());
                for value in zelf.odict.inner._as_dict_inner().values() {
                    let repr: PyStrRef = value.repr(vm)?;
                    str_parts.push(repr.as_str().to_owned());
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
        odict: PyOrderedDictRef,
    }

    #[pyclass(with(Iterable, Comparable, Representable))]
    impl PyOrderedDictItems {
        #[pymethod]
        fn __len__(&self) -> usize {
            self.odict.inner._as_dict_inner().len()
        }

        #[pymethod]
        fn __reversed__(&self) -> PyOrderedDictReverseItemIterator {
            PyOrderedDictReverseItemIterator::new(self.odict.clone())
        }
    }

    impl Iterable for PyOrderedDictItems {
        fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            Ok(PyOrderedDictItemIterator::new(zelf.odict.clone()).into_pyobject(vm))
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
                .odict
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
                let mut str_parts = Vec::with_capacity(zelf.odict.inner._as_dict_inner().len());
                for (key, value) in zelf.odict.inner._as_dict_inner().items() {
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
        fn new(odict: PyOrderedDictRef) -> Self {
            let size = odict.inner._as_dict_inner().size();
            Self {
                size,
                internal: PyMutex::new(PositionIterInternal::new(odict, 0)),
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
            let mut internal = zelf.internal.lock();
            let next = if let Active(odict) = &internal.status {
                if odict.inner._as_dict_inner().has_changed_size(&zelf.size) {
                    internal.status = Exhausted;
                    return Err(vm.new_runtime_error("dictionary changed size during iteration"));
                }
                match odict.inner._as_dict_inner().next_entry(internal.position) {
                    Some((position, key, _value)) => {
                        internal.position = position;
                        PyIterReturn::Return(key)
                    }
                    None => {
                        internal.status = Exhausted;
                        PyIterReturn::StopIteration(None)
                    }
                }
            } else {
                PyIterReturn::StopIteration(None)
            };
            Ok(next)
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
        fn new(odict: PyOrderedDictRef) -> Self {
            let size = odict.inner._as_dict_inner().size();
            Self {
                size,
                internal: PyMutex::new(PositionIterInternal::new(odict, 0)),
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
            let mut internal = zelf.internal.lock();
            let next = if let Active(odict) = &internal.status {
                if odict.inner._as_dict_inner().has_changed_size(&zelf.size) {
                    internal.status = Exhausted;
                    return Err(vm.new_runtime_error("dictionary changed size during iteration"));
                }
                match odict.inner._as_dict_inner().next_entry(internal.position) {
                    Some((position, _key, value)) => {
                        internal.position = position;
                        PyIterReturn::Return(value)
                    }
                    None => {
                        internal.status = Exhausted;
                        PyIterReturn::StopIteration(None)
                    }
                }
            } else {
                PyIterReturn::StopIteration(None)
            };
            Ok(next)
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
        fn new(odict: PyOrderedDictRef) -> Self {
            let size = odict.inner._as_dict_inner().size();
            Self {
                size,
                internal: PyMutex::new(PositionIterInternal::new(odict, 0)),
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
            let mut internal = zelf.internal.lock();
            let next = if let Active(odict) = &internal.status {
                if odict.inner._as_dict_inner().has_changed_size(&zelf.size) {
                    internal.status = Exhausted;
                    return Err(vm.new_runtime_error("dictionary changed size during iteration"));
                }
                match odict.inner._as_dict_inner().next_entry(internal.position) {
                    Some((position, key, value)) => {
                        internal.position = position;
                        PyIterReturn::Return(vm.new_tuple((key, value)).into())
                    }
                    None => {
                        internal.status = Exhausted;
                        PyIterReturn::StopIteration(None)
                    }
                }
            } else {
                PyIterReturn::StopIteration(None)
            };
            Ok(next)
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
        fn new(odict: PyOrderedDictRef) -> Self {
            let size = odict.inner._as_dict_inner().size();
            let position = size.entries_size.saturating_sub(1);
            Self {
                size,
                internal: PyMutex::new(PositionIterInternal::new(odict, position)),
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
            let mut internal = zelf.internal.lock();
            let next = if let Active(odict) = &internal.status {
                if odict.inner._as_dict_inner().has_changed_size(&zelf.size) {
                    internal.status = Exhausted;
                    return Err(vm.new_runtime_error("dictionary changed size during iteration"));
                }
                match odict.inner._as_dict_inner().prev_entry(internal.position) {
                    Some((position, key, _value)) => {
                        if internal.position == position {
                            internal.status = Exhausted;
                        } else {
                            internal.position = position;
                        }
                        PyIterReturn::Return(key)
                    }
                    None => {
                        internal.status = Exhausted;
                        PyIterReturn::StopIteration(None)
                    }
                }
            } else {
                PyIterReturn::StopIteration(None)
            };
            Ok(next)
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
        fn new(odict: PyOrderedDictRef) -> Self {
            let size = odict.inner._as_dict_inner().size();
            let position = size.entries_size.saturating_sub(1);
            Self {
                size,
                internal: PyMutex::new(PositionIterInternal::new(odict, position)),
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
            let mut internal = zelf.internal.lock();
            let next = if let Active(odict) = &internal.status {
                if odict.inner._as_dict_inner().has_changed_size(&zelf.size) {
                    internal.status = Exhausted;
                    return Err(vm.new_runtime_error("dictionary changed size during iteration"));
                }
                match odict.inner._as_dict_inner().prev_entry(internal.position) {
                    Some((position, _key, value)) => {
                        if internal.position == position {
                            internal.status = Exhausted;
                        } else {
                            internal.position = position;
                        }
                        PyIterReturn::Return(value)
                    }
                    None => {
                        internal.status = Exhausted;
                        PyIterReturn::StopIteration(None)
                    }
                }
            } else {
                PyIterReturn::StopIteration(None)
            };
            Ok(next)
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
        fn new(odict: PyOrderedDictRef) -> Self {
            let size = odict.inner._as_dict_inner().size();
            let position = size.entries_size.saturating_sub(1);
            Self {
                size,
                internal: PyMutex::new(PositionIterInternal::new(odict, position)),
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
            let mut internal = zelf.internal.lock();
            let next = if let Active(odict) = &internal.status {
                if odict.inner._as_dict_inner().has_changed_size(&zelf.size) {
                    internal.status = Exhausted;
                    return Err(vm.new_runtime_error("dictionary changed size during iteration"));
                }
                match odict.inner._as_dict_inner().prev_entry(internal.position) {
                    Some((position, key, value)) => {
                        if internal.position == position {
                            internal.status = Exhausted;
                        } else {
                            internal.position = position;
                        }
                        PyIterReturn::Return(vm.new_tuple((key, value)).into())
                    }
                    None => {
                        internal.status = Exhausted;
                        PyIterReturn::StopIteration(None)
                    }
                }
            } else {
                PyIterReturn::StopIteration(None)
            };
            Ok(next)
        }
    }
}
