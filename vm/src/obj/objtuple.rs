use std::cell::Cell;
use std::fmt;

use super::objiter;
use super::objsequence::{
    get_elements_tuple, get_item, seq_equal, seq_ge, seq_gt, seq_le, seq_lt, seq_mul,
};
use super::objtype::{self, PyClassRef};
use crate::function::OptionalArg;
use crate::pyhash;
use crate::pyobject::{
    IntoPyObject, PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue,
};
use crate::vm::{ReprGuard, VirtualMachine};

/// tuple() -> empty tuple
/// tuple(iterable) -> tuple initialized from iterable's items
///
/// If the argument is a tuple, the return value is the same object.
#[pyclass]
pub struct PyTuple {
    // TODO: shouldn't be public
    pub elements: Vec<PyObjectRef>,
}

impl fmt::Debug for PyTuple {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // TODO: implement more informational, non-recursive Debug formatter
        f.write_str("tuple")
    }
}

impl From<Vec<PyObjectRef>> for PyTuple {
    fn from(elements: Vec<PyObjectRef>) -> Self {
        PyTuple { elements }
    }
}

impl PyValue for PyTuple {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.tuple_type()
    }
}

macro_rules! impl_intopyobj_tuple {
    ($(($T:ident, $idx:tt)),+) => {
        impl<$($T: IntoPyObject),*> IntoPyObject for ($($T,)*) {
            fn into_pyobject(self, vm: &VirtualMachine) -> PyResult {
                Ok(vm.ctx.new_tuple(vec![$(self.$idx.into_pyobject(vm)?),*]))
            }
        }
    };
}

impl_intopyobj_tuple!((A, 0));
impl_intopyobj_tuple!((A, 0), (B, 1));
impl_intopyobj_tuple!((A, 0), (B, 1), (C, 2));
impl_intopyobj_tuple!((A, 0), (B, 1), (C, 2), (D, 3));
impl_intopyobj_tuple!((A, 0), (B, 1), (C, 2), (D, 3), (E, 4));
impl_intopyobj_tuple!((A, 0), (B, 1), (C, 2), (D, 3), (E, 4), (F, 5));
impl_intopyobj_tuple!((A, 0), (B, 1), (C, 2), (D, 3), (E, 4), (F, 5), (G, 6));

impl PyTuple {
    pub fn fast_getitem(&self, idx: usize) -> PyObjectRef {
        self.elements[idx].clone()
    }
}

pub type PyTupleRef = PyRef<PyTuple>;

pub fn get_value(obj: &PyObjectRef) -> Vec<PyObjectRef> {
    obj.payload::<PyTuple>().unwrap().elements.clone()
}

#[pyimpl]
impl PyTuple {
    #[pymethod(name = "__lt__")]
    fn lt(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if objtype::isinstance(&other, &vm.ctx.tuple_type()) {
            let other = get_elements_tuple(&other);
            let res = seq_lt(vm, &self.elements.as_slice(), &other.as_slice())?;
            Ok(vm.new_bool(res))
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    #[pymethod(name = "__gt__")]
    fn gt(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if objtype::isinstance(&other, &vm.ctx.tuple_type()) {
            let other = get_elements_tuple(&other);
            let res = seq_gt(vm, &self.elements.as_slice(), &other.as_slice())?;
            Ok(vm.new_bool(res))
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    #[pymethod(name = "__ge__")]
    fn ge(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if objtype::isinstance(&other, &vm.ctx.tuple_type()) {
            let other = get_elements_tuple(&other);
            let res = seq_ge(vm, &self.elements.as_slice(), &other.as_slice())?;
            Ok(vm.new_bool(res))
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    #[pymethod(name = "__le__")]
    fn le(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if objtype::isinstance(&other, &vm.ctx.tuple_type()) {
            let other = get_elements_tuple(&other);
            let res = seq_le(vm, &self.elements.as_slice(), &other.as_slice())?;
            Ok(vm.new_bool(res))
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    #[pymethod(name = "__add__")]
    fn add(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if objtype::isinstance(&other, &vm.ctx.tuple_type()) {
            let e2 = get_elements_tuple(&other);
            let elements = self.elements.iter().chain(e2.iter()).cloned().collect();
            Ok(vm.ctx.new_tuple(elements))
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    #[pymethod(name = "__bool__")]
    fn bool(&self, _vm: &VirtualMachine) -> bool {
        !self.elements.is_empty()
    }

    #[pymethod(name = "count")]
    fn count(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
        let mut count: usize = 0;
        for element in self.elements.iter() {
            if vm.identical_or_equal(element, &needle)? {
                count += 1;
            }
        }
        Ok(count)
    }

    #[pymethod(name = "__eq__")]
    fn eq(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if objtype::isinstance(&other, &vm.ctx.tuple_type()) {
            Ok(vm.new_bool(self.inner_eq(&other, vm)?))
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    #[pymethod(name = "__ne__")]
    fn ne(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if objtype::isinstance(&other, &vm.ctx.tuple_type()) {
            Ok(vm.new_bool(!self.inner_eq(&other, vm)?))
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    fn inner_eq(&self, other: &PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        let other = get_elements_tuple(other);
        seq_equal(vm, &self.elements.as_slice(), &other.as_slice())
    }

    #[pymethod(name = "__hash__")]
    fn hash(&self, vm: &VirtualMachine) -> PyResult<pyhash::PyHash> {
        pyhash::hash_iter(self.elements.iter(), vm)
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyTupleIterator {
        PyTupleIterator {
            position: Cell::new(0),
            tuple: zelf,
        }
    }

    #[pymethod(name = "__len__")]
    fn len(&self, _vm: &VirtualMachine) -> usize {
        self.elements.len()
    }

    #[pymethod(name = "__repr__")]
    fn repr(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<String> {
        let s = if let Some(_guard) = ReprGuard::enter(zelf.as_object()) {
            let mut str_parts = Vec::with_capacity(zelf.elements.len());
            for elem in zelf.elements.iter() {
                let s = vm.to_repr(elem)?;
                str_parts.push(s.as_str().to_owned());
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

    #[pymethod(name = "__mul__")]
    fn mul(&self, counter: isize, vm: &VirtualMachine) -> PyObjectRef {
        let new_elements = seq_mul(&self.elements.as_slice(), counter)
            .cloned()
            .collect();
        vm.ctx.new_tuple(new_elements)
    }

    #[pymethod(name = "__rmul__")]
    fn rmul(&self, counter: isize, vm: &VirtualMachine) -> PyObjectRef {
        self.mul(counter, vm)
    }

    #[pymethod(name = "__getitem__")]
    fn getitem(zelf: PyRef<Self>, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        get_item(vm, zelf.as_object(), &zelf.elements, needle.clone())
    }

    #[pymethod(name = "index")]
    fn index(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
        for (index, element) in self.elements.iter().enumerate() {
            if vm.identical_or_equal(element, &needle)? {
                return Ok(index);
            }
        }
        Err(vm.new_value_error("tuple.index(x): x not in tuple".to_string()))
    }

    #[pymethod(name = "__contains__")]
    fn contains(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        for element in self.elements.iter() {
            if vm.identical_or_equal(element, &needle)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    #[pyslot(new)]
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
        if self.position.get() < self.tuple.elements.len() {
            let ret = self.tuple.elements[self.position.get()].clone();
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
    let tuple_type = &context.types.tuple_type;
    PyTuple::extend_class(context, tuple_type);

    PyTupleIterator::extend_class(context, &context.types.tupleiterator_type);
}
