pub(crate) use _collections::make_module;

#[pymodule]
mod _collections {
    use crate::builtins::pytype::PyTypeRef;
    use crate::common::lock::{PyRwLock, PyRwLockReadGuard, PyRwLockWriteGuard};
    use crate::function::OptionalArg;
    use crate::pyobject::{
        PyComparisonValue, PyIterable, PyObjectRef, PyRef, PyResult, PyValue, StaticType,
    };
    use crate::slots::{Comparable, Iterable, PyComparisonOp, PyIter};
    use crate::vm::ReprGuard;
    use crate::VirtualMachine;
    use crate::{sequence, sliceable};
    use itertools::Itertools;
    use std::collections::VecDeque;

    use crossbeam_utils::atomic::AtomicCell;

    #[pyattr]
    #[pyclass(name = "deque")]
    #[derive(Debug)]
    struct PyDeque {
        deque: PyRwLock<VecDeque<PyObjectRef>>,
        maxlen: AtomicCell<Option<usize>>,
    }
    type PyDequeRef = PyRef<PyDeque>;

    impl PyValue for PyDeque {
        fn class(_vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
        }
    }

    #[derive(FromArgs)]
    struct PyDequeOptions {
        #[pyarg(any, default)]
        maxlen: Option<usize>,
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

    #[pyimpl(flags(BASETYPE), with(Comparable, Iterable))]
    impl PyDeque {
        #[pyslot]
        fn tp_new(
            cls: PyTypeRef,
            iter: OptionalArg<PyIterable>,
            PyDequeOptions { maxlen }: PyDequeOptions,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            let py_deque = PyDeque {
                deque: PyRwLock::default(),
                maxlen: AtomicCell::new(maxlen),
            };
            if let OptionalArg::Present(iter) = iter {
                py_deque.extend(iter, vm)?;
            }
            py_deque.into_ref_with_type(vm, cls)
        }

        #[pymethod]
        fn append(&self, obj: PyObjectRef) {
            let mut deque = self.borrow_deque_mut();
            if self.maxlen.load() == Some(deque.len()) {
                deque.pop_front();
            }
            deque.push_back(obj);
        }

        #[pymethod]
        fn appendleft(&self, obj: PyObjectRef) {
            let mut deque = self.borrow_deque_mut();
            if self.maxlen.load() == Some(deque.len()) {
                deque.pop_back();
            }
            deque.push_front(obj);
        }

        #[pymethod]
        fn clear(&self) {
            self.borrow_deque_mut().clear()
        }

        #[pymethod]
        fn copy(&self) -> Self {
            PyDeque {
                deque: PyRwLock::new(self.borrow_deque().clone()),
                maxlen: AtomicCell::new(self.maxlen.load()),
            }
        }

        #[pymethod]
        fn count(&self, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
            let mut count = 0;
            for elem in self.borrow_deque().iter() {
                if vm.identical_or_equal(elem, &obj)? {
                    count += 1;
                }
            }
            Ok(count)
        }

        #[pymethod]
        fn extend(&self, iter: PyIterable, vm: &VirtualMachine) -> PyResult<()> {
            // TODO: use length_hint here and for extendleft
            for elem in iter.iter(vm)? {
                self.append(elem?);
            }
            Ok(())
        }

        #[pymethod]
        fn extendleft(&self, iter: PyIterable, vm: &VirtualMachine) -> PyResult<()> {
            for elem in iter.iter(vm)? {
                self.appendleft(elem?);
            }
            Ok(())
        }

        #[pymethod]
        fn index(
            &self,
            obj: PyObjectRef,
            start: OptionalArg<usize>,
            stop: OptionalArg<usize>,
            vm: &VirtualMachine,
        ) -> PyResult<usize> {
            let deque = self.borrow_deque();
            let start = start.unwrap_or(0);
            let stop = stop.unwrap_or_else(|| deque.len());
            for (i, elem) in deque.iter().skip(start).take(stop - start).enumerate() {
                if vm.identical_or_equal(elem, &obj)? {
                    return Ok(i);
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
            let mut deque = self.borrow_deque_mut();

            if self.maxlen.load() == Some(deque.len()) {
                return Err(vm.new_index_error("deque already at its maximum size".to_owned()));
            }

            let idx = if idx < 0 {
                if -idx as usize > deque.len() {
                    0
                } else {
                    deque.len() - ((-idx) as usize)
                }
            } else if idx as usize >= deque.len() {
                deque.len() - 1
            } else {
                idx as usize
            };

            deque.insert(idx, obj);

            Ok(())
        }

        #[pymethod]
        fn pop(&self, vm: &VirtualMachine) -> PyResult {
            self.borrow_deque_mut()
                .pop_back()
                .ok_or_else(|| vm.new_index_error("pop from an empty deque".to_owned()))
        }

        #[pymethod]
        fn popleft(&self, vm: &VirtualMachine) -> PyResult {
            self.borrow_deque_mut()
                .pop_front()
                .ok_or_else(|| vm.new_index_error("pop from an empty deque".to_owned()))
        }

        #[pymethod]
        fn remove(&self, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            let mut deque = self.borrow_deque_mut();
            let mut idx = None;
            for (i, elem) in deque.iter().enumerate() {
                if vm.identical_or_equal(elem, &obj)? {
                    idx = Some(i);
                    break;
                }
            }
            idx.map(|idx| deque.remove(idx).unwrap())
                .ok_or_else(|| vm.new_value_error("deque.remove(x): x not in deque".to_owned()))
        }

        #[pymethod]
        fn reverse(&self) {
            let rev: VecDeque<_> = self.borrow_deque().iter().cloned().rev().collect();
            *self.borrow_deque_mut() = rev;
        }

        #[pymethod]
        fn rotate(&self, mid: OptionalArg<isize>) {
            let mut deque = self.borrow_deque_mut();
            let mid = mid.unwrap_or(1);
            if mid < 0 {
                deque.rotate_left(-mid as usize);
            } else {
                deque.rotate_right(mid as usize);
            }
        }

        #[pyproperty]
        fn maxlen(&self) -> Option<usize> {
            self.maxlen.load()
        }

        #[pyproperty(setter)]
        fn set_maxlen(&self, maxlen: Option<usize>) {
            self.maxlen.store(maxlen);
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
                let elements = zelf
                    .borrow_deque()
                    .iter()
                    .map(|obj| vm.to_repr(obj))
                    .collect::<Result<Vec<_>, _>>()?;
                let maxlen = zelf
                    .maxlen
                    .load()
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
            for element in self.borrow_deque().iter() {
                if vm.identical_or_equal(element, &needle)? {
                    return Ok(true);
                }
            }

            Ok(false)
        }

        #[pymethod(magic)]
        fn mul(&self, n: isize) -> Self {
            let deque: SimpleSeqDeque = self.borrow_deque().into();
            let mul = sequence::seq_mul(&deque, n);
            let skipped = if let Some(maxlen) = self.maxlen.load() {
                mul.len() - maxlen
            } else {
                0
            };
            let deque = mul.skip(skipped).cloned().collect();
            PyDeque {
                deque: PyRwLock::new(deque),
                maxlen: AtomicCell::new(self.maxlen.load()),
            }
        }

        #[pymethod(magic)]
        fn len(&self) -> usize {
            self.borrow_deque().len()
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

    impl Iterable for PyDeque {
        fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            Ok(PyDequeIterator {
                position: AtomicCell::new(0),
                deque: zelf,
            }
            .into_object(vm))
        }
    }

    #[pyattr]
    #[pyclass(name = "_deque_iterator")]
    #[derive(Debug)]
    struct PyDequeIterator {
        position: AtomicCell<usize>,
        deque: PyDequeRef,
    }

    impl PyValue for PyDequeIterator {
        fn class(_vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
        }
    }

    #[pyimpl(with(PyIter))]
    impl PyDequeIterator {}

    impl PyIter for PyDequeIterator {
        fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            let pos = zelf.position.fetch_add(1);
            let deque = zelf.deque.borrow_deque();
            if pos < deque.len() {
                let ret = deque[pos].clone();
                Ok(ret)
            } else {
                Err(vm.new_stop_iteration())
            }
        }
    }
}
