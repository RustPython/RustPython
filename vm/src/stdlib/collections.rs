use crate::function::OptionalArg;
use crate::obj::{objbool, objiter, objsequence, objtype::PyClassRef};
use crate::pyobject::{IdProtocol, PyClassImpl, PyIterable, PyObjectRef, PyRef, PyResult, PyValue};
use crate::vm::ReprGuard;
use crate::VirtualMachine;
use itertools::Itertools;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

#[pyclass(name = "deque")]
#[derive(Debug, Clone)]
struct PyDeque {
    deque: RefCell<VecDeque<PyObjectRef>>,
    maxlen: Cell<Option<usize>>,
}
type PyDequeRef = PyRef<PyDeque>;

impl PyValue for PyDeque {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("_collections", "deque")
    }
}

#[derive(FromArgs)]
struct PyDequeOptions {
    #[pyarg(positional_or_keyword, default = "None")]
    maxlen: Option<usize>,
}

#[pyimpl]
impl PyDeque {
    #[pyslot(new)]
    fn tp_new(
        cls: PyClassRef,
        iter: OptionalArg<PyIterable>,
        PyDequeOptions { maxlen }: PyDequeOptions,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<Self>> {
        let py_deque = PyDeque {
            deque: RefCell::default(),
            maxlen: maxlen.into(),
        };
        if let OptionalArg::Present(iter) = iter {
            py_deque.extend(iter, vm)?;
        }
        py_deque.into_ref_with_type(vm, cls)
    }

    #[pymethod]
    fn append(&self, obj: PyObjectRef, _vm: &VirtualMachine) {
        let mut deque = self.deque.borrow_mut();
        if self.maxlen.get() == Some(deque.len()) {
            deque.pop_front();
        }
        deque.push_back(obj);
    }

    #[pymethod]
    fn appendleft(&self, obj: PyObjectRef, _vm: &VirtualMachine) {
        let mut deque = self.deque.borrow_mut();
        if self.maxlen.get() == Some(deque.len()) {
            deque.pop_back();
        }
        deque.push_front(obj);
    }

    #[pymethod]
    fn clear(&self, _vm: &VirtualMachine) {
        self.deque.borrow_mut().clear()
    }

    #[pymethod]
    fn copy(&self, _vm: &VirtualMachine) -> Self {
        self.clone()
    }

    #[pymethod]
    fn count(&self, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
        let mut count = 0;
        for elem in self.deque.borrow().iter() {
            if objbool::boolval(vm, vm._eq(elem.clone(), obj.clone())?)? {
                count += 1;
            }
        }
        Ok(count)
    }

    #[pymethod]
    fn extend(&self, iter: PyIterable, vm: &VirtualMachine) -> PyResult<()> {
        // TODO: use length_hint here and for extendleft
        for elem in iter.iter(vm)? {
            self.append(elem?, vm);
        }
        Ok(())
    }

    #[pymethod]
    fn extendleft(&self, iter: PyIterable, vm: &VirtualMachine) -> PyResult<()> {
        for elem in iter.iter(vm)? {
            self.appendleft(elem?, vm);
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
        let deque = self.deque.borrow();
        let start = start.unwrap_or(0);
        let stop = stop.unwrap_or_else(|| deque.len());
        for (i, elem) in deque.iter().skip(start).take(stop - start).enumerate() {
            if objbool::boolval(vm, vm._eq(elem.clone(), obj.clone())?)? {
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
        let mut deque = self.deque.borrow_mut();

        if self.maxlen.get() == Some(deque.len()) {
            return Err(vm.new_index_error("deque already at its maximum size".to_string()));
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
        self.deque
            .borrow_mut()
            .pop_back()
            .ok_or_else(|| vm.new_index_error("pop from an empty deque".to_string()))
    }

    #[pymethod]
    fn popleft(&self, vm: &VirtualMachine) -> PyResult {
        self.deque
            .borrow_mut()
            .pop_front()
            .ok_or_else(|| vm.new_index_error("pop from an empty deque".to_string()))
    }

    #[pymethod]
    fn remove(&self, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let mut deque = self.deque.borrow_mut();
        let mut idx = None;
        for (i, elem) in deque.iter().enumerate() {
            if objbool::boolval(vm, vm._eq(elem.clone(), obj.clone())?)? {
                idx = Some(i);
                break;
            }
        }
        idx.map(|idx| deque.remove(idx).unwrap())
            .ok_or_else(|| vm.new_value_error("deque.remove(x): x not in deque".to_string()))
    }

    #[pymethod]
    fn reverse(&self, _vm: &VirtualMachine) {
        self.deque
            .replace_with(|deque| deque.iter().cloned().rev().collect());
    }

    #[pymethod]
    fn rotate(&self, mid: OptionalArg<isize>, _vm: &VirtualMachine) {
        let mut deque = self.deque.borrow_mut();
        let mid = mid.unwrap_or(1);
        if mid < 0 {
            deque.rotate_left(-mid as usize);
        } else {
            deque.rotate_right(mid as usize);
        }
    }

    #[pyproperty]
    fn maxlen(&self, _vm: &VirtualMachine) -> Option<usize> {
        self.maxlen.get()
    }
    #[pyproperty(setter)]
    fn set_maxlen(&self, maxlen: Option<usize>, vm: &VirtualMachine) -> PyResult {
        self.maxlen.set(maxlen);
        Ok(vm.get_none())
    }

    #[pymethod(name = "__repr__")]
    fn repr(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<String> {
        let repr = if let Some(_guard) = ReprGuard::enter(zelf.as_object()) {
            let elements = zelf
                .deque
                .borrow()
                .iter()
                .map(|obj| vm.to_repr(obj))
                .collect::<Result<Vec<_>, _>>()?;
            let maxlen = zelf
                .maxlen
                .get()
                .map(|maxlen| format!(", maxlen={}", maxlen))
                .unwrap_or_default();
            format!("deque([{}]{})", elements.into_iter().format(", "), maxlen)
        } else {
            "[...]".to_string()
        };
        Ok(repr)
    }

    #[pymethod(name = "__eq__")]
    fn eq(zelf: PyRef<Self>, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if zelf.as_object().is(&other) {
            return Ok(vm.new_bool(true));
        }

        let other = match_class!(match other {
            other @ Self => other,
            _ => return Ok(vm.ctx.not_implemented()),
        });

        let lhs: &VecDeque<_> = &zelf.deque.borrow();
        let rhs: &VecDeque<_> = &other.deque.borrow();

        let eq = objsequence::seq_equal(vm, lhs, rhs)?;
        Ok(vm.new_bool(eq))
    }

    #[pymethod(name = "__lt__")]
    fn lt(zelf: PyRef<Self>, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if zelf.as_object().is(&other) {
            return Ok(vm.new_bool(true));
        }

        let other = match_class!(match other {
            other @ Self => other,
            _ => return Ok(vm.ctx.not_implemented()),
        });

        let lhs: &VecDeque<_> = &zelf.deque.borrow();
        let rhs: &VecDeque<_> = &other.deque.borrow();

        let eq = objsequence::seq_lt(vm, lhs, rhs)?;
        Ok(vm.new_bool(eq))
    }

    #[pymethod(name = "__gt__")]
    fn gt(zelf: PyRef<Self>, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if zelf.as_object().is(&other) {
            return Ok(vm.new_bool(true));
        }

        let other = match_class!(match other {
            other @ Self => other,
            _ => return Ok(vm.ctx.not_implemented()),
        });

        let lhs: &VecDeque<_> = &zelf.deque.borrow();
        let rhs: &VecDeque<_> = &other.deque.borrow();

        let eq = objsequence::seq_gt(vm, lhs, rhs)?;
        Ok(vm.new_bool(eq))
    }

    #[pymethod(name = "__le__")]
    fn le(zelf: PyRef<Self>, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if zelf.as_object().is(&other) {
            return Ok(vm.new_bool(true));
        }

        let other = match_class!(match other {
            other @ Self => other,
            _ => return Ok(vm.ctx.not_implemented()),
        });

        let lhs: &VecDeque<_> = &zelf.deque.borrow();
        let rhs: &VecDeque<_> = &other.deque.borrow();

        let eq = objsequence::seq_le(vm, lhs, rhs)?;
        Ok(vm.new_bool(eq))
    }

    #[pymethod(name = "__ge__")]
    fn ge(zelf: PyRef<Self>, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if zelf.as_object().is(&other) {
            return Ok(vm.new_bool(true));
        }

        let other = match_class!(match other {
            other @ Self => other,
            _ => return Ok(vm.ctx.not_implemented()),
        });

        let lhs: &VecDeque<_> = &zelf.deque.borrow();
        let rhs: &VecDeque<_> = &other.deque.borrow();

        let eq = objsequence::seq_ge(vm, lhs, rhs)?;
        Ok(vm.new_bool(eq))
    }

    #[pymethod(name = "__mul__")]
    fn mul(&self, n: isize, _vm: &VirtualMachine) -> Self {
        let deque: &VecDeque<_> = &self.deque.borrow();
        let mul = objsequence::seq_mul(deque, n);
        let skipped = if let Some(maxlen) = self.maxlen.get() {
            mul.len() - maxlen
        } else {
            0
        };
        let deque = mul.skip(skipped).cloned().collect();
        PyDeque {
            deque: RefCell::new(deque),
            maxlen: self.maxlen.clone(),
        }
    }

    #[pymethod(name = "__len__")]
    fn len(&self, _vm: &VirtualMachine) -> usize {
        self.deque.borrow().len()
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyDequeIterator {
        PyDequeIterator {
            position: Cell::new(0),
            deque: zelf,
        }
    }
}

#[pyclass(name = "_deque_iterator")]
#[derive(Debug)]
struct PyDequeIterator {
    position: Cell<usize>,
    deque: PyDequeRef,
}

impl PyValue for PyDequeIterator {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("_collections", "_deque_iterator")
    }
}

#[pyimpl]
impl PyDequeIterator {
    #[pymethod(name = "__next__")]
    fn next(&self, vm: &VirtualMachine) -> PyResult {
        if self.position.get() < self.deque.deque.borrow().len() {
            let ret = self.deque.deque.borrow()[self.position.get()].clone();
            self.position.set(self.position.get() + 1);
            Ok(ret)
        } else {
            Err(objiter::new_stop_iteration(vm))
        }
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyRef<Self> {
        zelf
    }
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    py_module!(vm, "_collections", {
        "deque" => PyDeque::make_class(&vm.ctx),
        "_deque_iterator" => PyDequeIterator::make_class(&vm.ctx),
    })
}
