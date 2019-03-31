use std::cell::RefCell;
use std::fmt;
use std::hash::{Hash, Hasher};

use crate::function::OptionalArg;
use crate::pyobject::{PyContext, PyObject, PyObjectRef, PyRef, PyResult, PyValue};
use crate::vm::{ReprGuard, VirtualMachine};

use super::objint;
use super::objsequence::{get_elements, SequenceProtocol};
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

impl SequenceProtocol for PyTupleRef {
    fn get_elements(&self) -> Vec<PyObjectRef> {
        self.elements.borrow().clone()
    }
    fn create(&self, vm: &VirtualMachine, elements: Vec<PyObjectRef>) -> PyResult {
        Ok(PyObject::new(
            PyTuple {
                elements: RefCell::new(elements),
            },
            PyTuple::class(vm),
            None,
        ))
    }
    fn as_object(&self) -> &PyObjectRef {
        self.as_object()
    }
    fn into_object(self) -> PyObjectRef {
        self.into_object()
    }
    fn class(&self) -> PyClassRef {
        self.typ()
    }
}

impl PyTupleRef {
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

    fn hash(self, vm: &VirtualMachine) -> PyResult<u64> {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for element in self.elements.borrow().iter() {
            let hash_result = vm.call_method(element, "__hash__", vec![])?;
            let element_hash = objint::get_value(&hash_result);
            element_hash.hash(&mut hasher);
        }
        Ok(hasher.finish())
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
}
