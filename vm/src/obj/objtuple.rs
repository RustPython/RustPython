use std::cell::{Cell, RefCell};
use std::fmt;
use std::hash::{Hash, Hasher};

use crate::function::OptionalArg;
use crate::pyobject::{IdProtocol, PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue};
use crate::vm::{ReprGuard, VirtualMachine};

use super::objbool;
use super::objint;
use super::objiter;
use super::objsequence::{
    get_elements, get_item, seq_equal, seq_ge, seq_gt, seq_le, seq_lt, seq_mul,
};
use super::objtype::{self, PyClassRef};

pub struct PyTuple {
    // TODO: shouldn't be public
    // TODO: tuples are immutable, remove this RefCell
    pub elements: RefCell<Vec<PyObjectRef>>,
}

impl fmt::Debug for PyTuple {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // TODO: implement more informational, non-recursive Debug formatter
        f.write_str("tuple")
    }
}

impl From<Vec<PyObjectRef>> for PyTuple {
    fn from(elements: Vec<PyObjectRef>) -> Self {
        PyTuple {
            elements: RefCell::new(elements),
        }
    }
}

impl PyValue for PyTuple {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.tuple_type()
    }
}

pub type PyTupleRef = PyRef<PyTuple>;

impl PyTupleRef {
    fn lt(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if objtype::isinstance(&other, &vm.ctx.tuple_type()) {
            let zelf = self.elements.borrow();
            let other = get_elements(&other);
            let res = seq_lt(vm, &zelf, &other)?;
            Ok(vm.new_bool(res))
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    fn gt(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if objtype::isinstance(&other, &vm.ctx.tuple_type()) {
            let zelf = self.elements.borrow();
            let other = get_elements(&other);
            let res = seq_gt(vm, &zelf, &other)?;
            Ok(vm.new_bool(res))
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    fn ge(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if objtype::isinstance(&other, &vm.ctx.tuple_type()) {
            let zelf = self.elements.borrow();
            let other = get_elements(&other);
            let res = seq_ge(vm, &zelf, &other)?;
            Ok(vm.new_bool(res))
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    fn le(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if objtype::isinstance(&other, &vm.ctx.tuple_type()) {
            let zelf = self.elements.borrow();
            let other = get_elements(&other);
            let res = seq_le(vm, &zelf, &other)?;
            Ok(vm.new_bool(res))
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    fn add(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if objtype::isinstance(&other, &vm.ctx.tuple_type()) {
            let e1 = self.elements.borrow();
            let e2 = get_elements(&other);
            let elements = e1.iter().chain(e2.iter()).cloned().collect();
            Ok(vm.ctx.new_tuple(elements))
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    fn bool(self, _vm: &VirtualMachine) -> bool {
        !self.elements.borrow().is_empty()
    }

    fn count(self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
        let mut count: usize = 0;
        for element in self.elements.borrow().iter() {
            if element.is(&needle) {
                count += 1;
            } else {
                let is_eq = vm._eq(element.clone(), needle.clone())?;
                if objbool::boolval(vm, is_eq)? {
                    count += 1;
                }
            }
        }
        Ok(count)
    }

    fn eq(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if objtype::isinstance(&other, &vm.ctx.tuple_type()) {
            let zelf = &self.elements.borrow();
            let other = get_elements(&other);
            let res = seq_equal(vm, &zelf, &other)?;
            Ok(vm.new_bool(res))
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    fn hash(self, vm: &VirtualMachine) -> PyResult<u64> {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for element in self.elements.borrow().iter() {
            let hash_result = vm.call_method(element, "__hash__", vec![])?;
            let element_hash = objint::get_value(&hash_result);
            element_hash.hash(&mut hasher);
        }
        Ok(hasher.finish())
    }

    fn iter(self, _vm: &VirtualMachine) -> PyTupleIterator {
        PyTupleIterator {
            position: Cell::new(0),
            tuple: self,
        }
    }

    fn len(self, _vm: &VirtualMachine) -> usize {
        self.elements.borrow().len()
    }

    fn repr(self, vm: &VirtualMachine) -> PyResult<String> {
        let s = if let Some(_guard) = ReprGuard::enter(self.as_object()) {
            let mut str_parts = vec![];
            for elem in self.elements.borrow().iter() {
                let s = vm.to_repr(elem)?;
                str_parts.push(s.value.clone());
            }

            if str_parts.len() == 1 {
                format!("({},)", str_parts[0])
            } else {
                format!("({})", str_parts.join(", "))
            }
        } else {
            "(...)".to_string()
        };
        Ok(s)
    }

    fn mul(self, counter: isize, vm: &VirtualMachine) -> PyObjectRef {
        let new_elements = seq_mul(&self.elements.borrow(), counter);
        vm.ctx.new_tuple(new_elements)
    }

    fn getitem(self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        get_item(
            vm,
            self.as_object(),
            &self.elements.borrow(),
            needle.clone(),
        )
    }

    fn index(self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
        for (index, element) in self.elements.borrow().iter().enumerate() {
            if element.is(&needle) {
                return Ok(index);
            }
            let is_eq = vm._eq(needle.clone(), element.clone())?;
            if objbool::boolval(vm, is_eq)? {
                return Ok(index);
            }
        }
        Err(vm.new_value_error("tuple.index(x): x not in tuple".to_string()))
    }

    fn contains(self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        for element in self.elements.borrow().iter() {
            if element.is(&needle) {
                return Ok(true);
            }
            let is_eq = vm._eq(needle.clone(), element.clone())?;
            if objbool::boolval(vm, is_eq)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

fn tuple_new(
    cls: PyClassRef,
    iterable: OptionalArg<PyObjectRef>,
    vm: &VirtualMachine,
) -> PyResult<PyTupleRef> {
    let elements = if let OptionalArg::Present(iterable) = iterable {
        vm.extract_elements(&iterable)?
    } else {
        vec![]
    };

    PyTuple::from(elements).into_ref_with_type(vm, cls)
}

#[pyclass]
#[derive(Debug)]
pub struct PyTupleIterator {
    position: Cell<usize>,
    tuple: PyTupleRef,
}

impl PyValue for PyTupleIterator {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.tupleiterator_type()
    }
}

#[pyimpl]
impl PyTupleIterator {
    #[pymethod(name = "__next__")]
    fn next(&self, vm: &VirtualMachine) -> PyResult {
        if self.position.get() < self.tuple.elements.borrow().len() {
            let ret = self.tuple.elements.borrow()[self.position.get()].clone();
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

#[rustfmt::skip] // to avoid line splitting
pub fn init(context: &PyContext) {
    let tuple_type = &context.tuple_type;
    let tuple_doc = "tuple() -> empty tuple
tuple(iterable) -> tuple initialized from iterable's items

If the argument is a tuple, the return value is the same object.";
    extend_class!(context, tuple_type, {
        "__add__" => context.new_rustfunc(PyTupleRef::add),
        "__bool__" => context.new_rustfunc(PyTupleRef::bool),
        "__eq__" => context.new_rustfunc(PyTupleRef::eq),
        "__contains__" => context.new_rustfunc(PyTupleRef::contains),
        "__getitem__" => context.new_rustfunc(PyTupleRef::getitem),
        "__hash__" => context.new_rustfunc(PyTupleRef::hash),
        "__iter__" => context.new_rustfunc(PyTupleRef::iter),
        "__len__" => context.new_rustfunc(PyTupleRef::len),
        "__new__" => context.new_rustfunc(tuple_new),
        "__mul__" => context.new_rustfunc(PyTupleRef::mul),
        "__repr__" => context.new_rustfunc(PyTupleRef::repr),
        "count" => context.new_rustfunc(PyTupleRef::count),
        "__lt__" => context.new_rustfunc(PyTupleRef::lt),
        "__le__" => context.new_rustfunc(PyTupleRef::le),
        "__gt__" => context.new_rustfunc(PyTupleRef::gt),
        "__ge__" => context.new_rustfunc(PyTupleRef::ge),
        "__doc__" => context.new_str(tuple_doc.to_string()),
        "index" => context.new_rustfunc(PyTupleRef::index)
    });

    PyTupleIterator::extend_class(context, &context.tupleiterator_type);
}
