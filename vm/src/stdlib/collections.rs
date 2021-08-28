pub(crate) use _collections::make_module;

#[pymodule]
mod _collections {
    use crate::common::lock::{PyRwLock, PyRwLockReadGuard, PyRwLockWriteGuard};
    use crate::function::{FuncArgs, OptionalArg};
    use crate::slots::{Comparable, Hashable, Iterable, PyComparisonOp, PyIter, Unhashable};
    use crate::vm::ReprGuard;
    use crate::VirtualMachine;
    use crate::{
        builtins::{
            IterStatus::{self, Active, Exhausted},
            PyInt, PyTypeRef,
        },
        TypeProtocol,
    };
    use crate::{sequence, sliceable};
    use crate::{PyComparisonValue, PyObjectRef, PyRef, PyResult, PyValue, StaticType};
    use crossbeam_utils::atomic::AtomicCell;
    use itertools::Itertools;
    use num_traits::ToPrimitive;
    use std::cmp::{max, min};
    use std::collections::VecDeque;

    #[pyattr]
    #[pyclass(name = "deque")]
    #[derive(Debug, Default)]
    struct PyDeque {
        deque: PyRwLock<VecDeque<PyObjectRef>>,
        maxlen: Option<usize>,
        state: AtomicCell<usize>, // incremented whenever the indices move
    }

    type PyDequeRef = PyRef<PyDeque>;

    impl PyValue for PyDeque {
        fn class(_vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
        }
    }

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
        fn tp_new(cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
            PyDeque::default().into_ref_with_type(vm, cls)
        }

        #[pymethod(name = "__init__")]
        fn init(
            zelf: PyRef<Self>,
            PyDequeOptions { iterable, maxlen }: PyDequeOptions,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            // TODO: This is _basically_ pyobject_to_opt_usize in itertools.rs
            // need to move that function elsewhere and refactor usages.
            let maxlen = if let Some(obj) = maxlen.into_option() {
                if !vm.is_none(&obj) {
                    let value = obj.payload::<PyInt>().ok_or_else(|| {
                        vm.new_value_error("maxlen must be non-negative.".to_owned())
                    })?;
                    let maxlen = value.as_bigint().to_usize().ok_or_else(|| {
                        vm.new_value_error("maxlen must be non-negative.".to_owned())
                    })?;
                    // Only succeeds for values for which 0 <= value <= isize::MAX
                    if maxlen > isize::MAX as usize {
                        return Err(vm.new_overflow_error(
                            "Python int too large to convert to Rust isize.".to_owned(),
                        ));
                    }
                    Some(maxlen)
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
            // TODO: use length_hint here and for extendleft
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
        fn reversed(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            let length = zelf.len();
            Ok(PyReverseDequeIterator {
                position: AtomicCell::new(length),
                status: AtomicCell::new(if length > 0 { Active } else { Exhausted }),
                length,
                deque: zelf,
            }
            .into_object(vm))
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
                format!("deque([{}]{})", elements.into_iter().format(", "), maxlen)
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
        fn mul(&self, n: isize) -> Self {
            let deque: SimpleSeqDeque = self.borrow_deque().into();
            let mul = sequence::seq_mul(&deque, n);
            let skipped = self
                .maxlen
                .and_then(|maxlen| mul.len().checked_sub(maxlen))
                .unwrap_or(0);

            let deque = mul.skip(skipped).cloned().collect();
            PyDeque {
                deque: PyRwLock::new(deque),
                maxlen: self.maxlen,
                state: AtomicCell::new(0),
            }
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
                    other.class().name
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
    #[derive(Debug)]
    struct PyDequeIterator {
        position: AtomicCell<usize>,
        status: AtomicCell<IterStatus>,
        length: usize, // To track length immutability.
        deque: PyDequeRef,
    }

    impl PyValue for PyDequeIterator {
        fn class(_vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
        }
    }

    #[pyimpl(with(PyIter))]
    impl PyDequeIterator {
        pub(crate) fn new(deque: PyDequeRef) -> Self {
            PyDequeIterator {
                position: AtomicCell::new(0),
                status: AtomicCell::new(IterStatus::Active),
                length: deque.len(),
                deque,
            }
        }

        #[pymethod(magic)]
        fn length_hint(&self) -> usize {
            match self.status.load() {
                Active => self.deque.len().saturating_sub(self.position.load()),
                Exhausted => 0,
            }
        }

        #[pymethod(magic)]
        fn reduce(
            zelf: PyRef<Self>,
            vm: &VirtualMachine,
        ) -> PyResult<(PyTypeRef, (PyDequeRef, PyObjectRef))> {
            Ok((
                zelf.clone_class(),
                (zelf.deque.clone(), vm.ctx.new_int(zelf.position.load())),
            ))
        }
    }

    impl PyIter for PyDequeIterator {
        fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            match zelf.status.load() {
                Exhausted => Err(vm.new_stop_iteration()),
                Active => {
                    if zelf.length != zelf.deque.len() {
                        // Deque was changed while we iterated.
                        zelf.status.store(Exhausted);
                        Err(vm.new_runtime_error("Deque mutated during iteration".to_owned()))
                    } else {
                        let pos = zelf.position.fetch_add(1);
                        let deque = zelf.deque.borrow_deque();
                        if pos < deque.len() {
                            let ret = deque[pos].clone();
                            Ok(ret)
                        } else {
                            zelf.status.store(Exhausted);
                            Err(vm.new_stop_iteration())
                        }
                    }
                }
            }
        }
    }

    #[pyattr]
    #[pyclass(name = "_deque_reverse_iterator")]
    #[derive(Debug)]
    struct PyReverseDequeIterator {
        position: AtomicCell<usize>,
        status: AtomicCell<IterStatus>,
        length: usize, // To track length immutability.
        deque: PyDequeRef,
    }

    impl PyValue for PyReverseDequeIterator {
        fn class(_vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
        }
    }

    #[pyimpl(with(PyIter))]
    impl PyReverseDequeIterator {
        #[pymethod(magic)]
        fn length_hint(&self) -> usize {
            match self.status.load() {
                Active => self.position.load(),
                Exhausted => 0,
            }
        }

        #[pymethod(magic)]
        fn reduce(
            zelf: PyRef<Self>,
            vm: &VirtualMachine,
        ) -> PyResult<(PyTypeRef, (PyDequeRef, PyObjectRef))> {
            Ok((
                zelf.clone_class(),
                (zelf.deque.clone(), vm.ctx.new_int(zelf.position.load())),
            ))
        }
    }

    impl PyIter for PyReverseDequeIterator {
        fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            match zelf.status.load() {
                Exhausted => Err(vm.new_stop_iteration()),
                Active => {
                    // If length changes while we iterate, set to Exhausted and bail.
                    if zelf.length != zelf.deque.len() {
                        zelf.status.store(Exhausted);
                        Err(vm.new_runtime_error("Deque mutated during iteration".to_owned()))
                    } else {
                        let pos = zelf.position.fetch_sub(1) - 1;
                        let deque = zelf.deque.borrow_deque();
                        if pos > 0 {
                            if let Some(obj) = deque.get(pos) {
                                return Ok(obj.clone());
                            }
                        }
                        // We either are == 0 or deque.get returned None. Either way, set status
                        // to exhausted and return last item if pos == 0.
                        zelf.status.store(Exhausted);
                        if pos == 0 {
                            // Can safely index directly.
                            return Ok(deque[pos].clone());
                        }
                        Err(vm.new_stop_iteration())
                    }
                }
            }
        }
    }
}
