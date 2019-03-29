/*
 * Builtin set type with a sequence of unique items.
 */

use std::cell::{Cell, RefCell};
use std::collections::{hash_map::DefaultHasher, HashMap};
use std::fmt;
use std::hash::{Hash, Hasher};

use crate::function::OptionalArg;
use crate::pyobject::{
    PyContext, PyIteratorValue, PyObject, PyObjectRef, PyRef, PyResult, PyValue, TypeProtocol,
};
use crate::vm::{ReprGuard, VirtualMachine};

use super::objbool;
use super::objint;
use super::objiter;
use super::objtype;
use super::objtype::PyClassRef;

#[derive(Default)]
pub struct PySet {
    elements: RefCell<HashMap<u64, PyObjectRef>>,
}
pub type PySetRef = PyRef<PySet>;

#[derive(Default)]
pub struct PyFrozenSet {
    elements: HashMap<u64, PyObjectRef>,
}
pub type PyFrozenSetRef = PyRef<PyFrozenSet>;

impl fmt::Debug for PySet {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // TODO: implement more detailed, non-recursive Debug formatter
        f.write_str("set")
    }
}

impl fmt::Debug for PyFrozenSet {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // TODO: implement more detailed, non-recursive Debug formatter
        f.write_str("frozenset")
    }
}

impl PyValue for PySet {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.set_type()
    }
}

impl PyValue for PyFrozenSet {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.frozenset_type()
    }
}

trait SetProtocol
where
    Self: Sized,
{
    fn get_elements(&self) -> HashMap<u64, PyObjectRef>;
    fn as_object(&self) -> &PyObjectRef;
    fn create(&self, vm: &VirtualMachine, elements: HashMap<u64, PyObjectRef>) -> PyResult;
    fn name(&self) -> &str;
    fn len(self, _vm: &VirtualMachine) -> usize {
        self.get_elements().len()
    }
    fn copy(self, vm: &VirtualMachine) -> PyResult {
        self.create(vm, self.get_elements())
    }
    fn contains(self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        for element in self.get_elements().iter() {
            match vm._eq(needle.clone(), element.1.clone()) {
                Ok(value) => {
                    if objbool::get_value(&value) {
                        return Ok(vm.new_bool(true));
                    }
                }
                Err(_) => return Err(vm.new_type_error("".to_string())),
            }
        }
        Ok(vm.new_bool(false))
    }

    fn eq(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        set_compare_inner(
            vm,
            self.as_object(),
            &other,
            &|zelf: usize, other: usize| -> bool { zelf != other },
            false,
        )
    }

    fn ge(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        set_compare_inner(
            vm,
            self.as_object(),
            &other,
            &|zelf: usize, other: usize| -> bool { zelf < other },
            false,
        )
    }

    fn gt(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        set_compare_inner(
            vm,
            self.as_object(),
            &other,
            &|zelf: usize, other: usize| -> bool { zelf <= other },
            false,
        )
    }

    fn le(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        set_compare_inner(
            vm,
            self.as_object(),
            &other,
            &|zelf: usize, other: usize| -> bool { zelf < other },
            true,
        )
    }

    fn lt(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        set_compare_inner(
            vm,
            self.as_object(),
            &other,
            &|zelf: usize, other: usize| -> bool { zelf <= other },
            true,
        )
    }

    fn union(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        validate_set_or_frozenset(vm, other.class())?;

        let mut elements = self.get_elements().clone();
        elements.extend(get_elements(&other).clone());

        self.create(vm, elements)
    }

    fn intersection(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.combine_inner(&other, vm, SetCombineOperation::Intersection)
    }

    fn difference(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.combine_inner(&other, vm, SetCombineOperation::Difference)
    }

    fn combine_inner(
        self,
        other: &PyObjectRef,
        vm: &VirtualMachine,
        op: SetCombineOperation,
    ) -> PyResult {
        validate_set_or_frozenset(vm, other.class())?;
        let mut elements = HashMap::new();

        for element in self.get_elements().iter() {
            let value = vm.call_method(other, "__contains__", vec![element.1.clone()])?;
            let should_add = match op {
                SetCombineOperation::Intersection => objbool::get_value(&value),
                SetCombineOperation::Difference => !objbool::get_value(&value),
            };
            if should_add {
                elements.insert(element.0.clone(), element.1.clone());
            }
        }

        self.create(vm, elements)
    }

    fn symmetric_difference(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        validate_set_or_frozenset(vm, other.class())?;
        let mut elements = HashMap::new();

        for element in self.get_elements().iter() {
            let value = vm.call_method(&other, "__contains__", vec![element.1.clone()])?;
            if !objbool::get_value(&value) {
                elements.insert(element.0.clone(), element.1.clone());
            }
        }

        for element in get_elements(&other).iter() {
            let value =
                vm.call_method(self.as_object(), "__contains__", vec![element.1.clone()])?;
            if !objbool::get_value(&value) {
                elements.insert(element.0.clone(), element.1.clone());
            }
        }

        self.create(vm, elements)
    }

    fn iter(self, vm: &VirtualMachine) -> PyIteratorValue {
        let items = self.get_elements().values().cloned().collect();
        let set_list = vm.ctx.new_list(items);
        PyIteratorValue {
            position: Cell::new(0),
            iterated_obj: set_list,
        }
    }

    fn repr(self, vm: &VirtualMachine) -> PyResult {
        let elements = self.get_elements();
        let s = if elements.is_empty() {
            format!("{}()", self.name())
        } else if let Some(_guard) = ReprGuard::enter(self.as_object()) {
            let mut str_parts = vec![];
            for elem in elements.values() {
                let part = vm.to_repr(elem)?;
                str_parts.push(part.value.clone());
            }

            format!("{{{}}}", str_parts.join(", "))
        } else {
            format!("{}(...)", self.name())
        };
        Ok(vm.new_str(s))
    }
}

impl SetProtocol for PySetRef {
    fn get_elements(&self) -> HashMap<u64, PyObjectRef> {
        self.elements.borrow().clone()
    }
    fn create(&self, vm: &VirtualMachine, elements: HashMap<u64, PyObjectRef>) -> PyResult {
        Ok(PyObject::new(
            PySet {
                elements: RefCell::new(elements),
            },
            PySet::class(vm),
            None,
        ))
    }
    fn as_object(&self) -> &PyObjectRef {
        self.as_object()
    }
    fn name(&self) -> &str {
        "set"
    }
}

impl SetProtocol for PyFrozenSetRef {
    fn get_elements(&self) -> HashMap<u64, PyObjectRef> {
        self.elements.clone()
    }
    fn create(&self, vm: &VirtualMachine, elements: HashMap<u64, PyObjectRef>) -> PyResult {
        Ok(PyObject::new(
            PyFrozenSet { elements: elements },
            PyFrozenSet::class(vm),
            None,
        ))
    }
    fn as_object(&self) -> &PyObjectRef {
        self.as_object()
    }
    fn name(&self) -> &str {
        "frozenset"
    }
}

impl PySetRef {
    fn add(self, item: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        insert_into_set(vm, &mut self.elements.borrow_mut(), &item)
    }

    fn remove(self, item: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        fn remove(
            vm: &VirtualMachine,
            elements: &mut HashMap<u64, PyObjectRef>,
            key: u64,
            value: &PyObjectRef,
        ) -> PyResult {
            match elements.remove(&key) {
                None => {
                    let item_str = format!("{:?}", value);
                    Err(vm.new_key_error(item_str))
                }
                Some(_) => Ok(vm.get_none()),
            }
        }
        perform_action_with_hash(vm, &mut self.elements.borrow_mut(), &item, &remove)
    }

    fn discard(self, item: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        fn discard(
            vm: &VirtualMachine,
            elements: &mut HashMap<u64, PyObjectRef>,
            key: u64,
            _value: &PyObjectRef,
        ) -> PyResult {
            elements.remove(&key);
            Ok(vm.get_none())
        }
        perform_action_with_hash(vm, &mut self.elements.borrow_mut(), &item, &discard)
    }

    fn clear(self, vm: &VirtualMachine) -> PyResult {
        self.elements.borrow_mut().clear();
        Ok(vm.get_none())
    }

    fn pop(self, vm: &VirtualMachine) -> PyResult {
        let mut elements = self.elements.borrow_mut();
        match elements.clone().keys().next() {
            Some(key) => Ok(elements.remove(key).unwrap()),
            None => Err(vm.new_key_error("pop from an empty set".to_string())),
        }
    }

    fn ior(self, iterable: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let iterator = objiter::get_iter(vm, &iterable)?;
        while let Ok(v) = vm.call_method(&iterator, "__next__", vec![]) {
            insert_into_set(vm, &mut self.elements.borrow_mut(), &v)?;
        }
        Ok(self.as_object().clone())
    }

    fn update(self, iterable: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.ior(iterable, vm)?;
        Ok(vm.get_none())
    }

    fn intersection_update(self, iterable: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.combine_update_inner(&iterable, vm, SetCombineOperation::Intersection)?;
        Ok(vm.get_none())
    }

    fn iand(self, iterable: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.combine_update_inner(&iterable, vm, SetCombineOperation::Intersection)
    }

    fn difference_update(self, iterable: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.combine_update_inner(&iterable, vm, SetCombineOperation::Difference)?;
        Ok(vm.get_none())
    }

    fn isub(self, iterable: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.combine_update_inner(&iterable, vm, SetCombineOperation::Difference)
    }

    fn combine_update_inner(
        self,
        iterable: &PyObjectRef,
        vm: &VirtualMachine,
        op: SetCombineOperation,
    ) -> PyResult {
        let mut elements = self.elements.borrow_mut();
        for element in elements.clone().iter() {
            let value = vm.call_method(iterable, "__contains__", vec![element.1.clone()])?;
            let should_remove = match op {
                SetCombineOperation::Intersection => !objbool::get_value(&value),
                SetCombineOperation::Difference => objbool::get_value(&value),
            };
            if should_remove {
                elements.remove(&element.0.clone());
            }
        }
        Ok(self.as_object().clone())
    }

    fn symmetric_difference_update(self, iterable: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.ixor(iterable, vm)?;
        Ok(vm.get_none())
    }

    fn ixor(self, iterable: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let elements_original = self.elements.borrow().clone();
        let iterator = objiter::get_iter(vm, &iterable)?;
        while let Ok(v) = vm.call_method(&iterator, "__next__", vec![]) {
            insert_into_set(vm, &mut self.elements.borrow_mut(), &v)?;
        }
        for element in elements_original.iter() {
            let value = vm.call_method(&iterable, "__contains__", vec![element.1.clone()])?;
            if objbool::get_value(&value) {
                self.elements.borrow_mut().remove(&element.0.clone());
            }
        }

        Ok(self.as_object().clone())
    }
}

pub fn get_elements(obj: &PyObjectRef) -> HashMap<u64, PyObjectRef> {
    if let Some(set) = obj.payload::<PySet>() {
        return set.elements.borrow().clone();
    } else if let Some(frozenset) = obj.payload::<PyFrozenSet>() {
        return frozenset.elements.clone();
    }
    panic!("Not frozenset or set");
}

fn validate_set_or_frozenset(vm: &VirtualMachine, cls: PyClassRef) -> PyResult<()> {
    if !(objtype::issubclass(&cls, &vm.ctx.set_type())
        || objtype::issubclass(&cls, &vm.ctx.frozenset_type()))
    {
        return Err(vm.new_type_error(format!("{} is not a subtype of set or frozenset", cls)));
    }
    Ok(())
}

fn create_set(
    vm: &VirtualMachine,
    elements: HashMap<u64, PyObjectRef>,
    cls: PyClassRef,
) -> PyResult {
    if objtype::issubclass(&cls, &vm.ctx.set_type()) {
        Ok(PyObject::new(
            PySet {
                elements: RefCell::new(elements),
            },
            PySet::class(vm),
            None,
        ))
    } else if objtype::issubclass(&cls, &vm.ctx.frozenset_type()) {
        Ok(PyObject::new(
            PyFrozenSet { elements: elements },
            PyFrozenSet::class(vm),
            None,
        ))
    } else {
        Err(vm.new_type_error(format!("{} is not a subtype of set or frozenset", cls)))
    }
}

fn perform_action_with_hash(
    vm: &VirtualMachine,
    elements: &mut HashMap<u64, PyObjectRef>,
    item: &PyObjectRef,
    f: &Fn(&VirtualMachine, &mut HashMap<u64, PyObjectRef>, u64, &PyObjectRef) -> PyResult,
) -> PyResult {
    let hash: PyObjectRef = vm.call_method(item, "__hash__", vec![])?;

    let hash_value = objint::get_value(&hash);
    let mut hasher = DefaultHasher::new();
    hash_value.hash(&mut hasher);
    let key = hasher.finish();
    f(vm, elements, key, item)
}

fn insert_into_set(
    vm: &VirtualMachine,
    elements: &mut HashMap<u64, PyObjectRef>,
    item: &PyObjectRef,
) -> PyResult {
    fn insert(
        vm: &VirtualMachine,
        elements: &mut HashMap<u64, PyObjectRef>,
        key: u64,
        value: &PyObjectRef,
    ) -> PyResult {
        elements.insert(key, value.clone());
        Ok(vm.get_none())
    }
    perform_action_with_hash(vm, elements, item, &insert)
}

/* Create a new object of sub-type of set */
fn set_new(cls: PyClassRef, iterable: OptionalArg<PyObjectRef>, vm: &VirtualMachine) -> PyResult {
    validate_set_or_frozenset(vm, cls.clone())?;

    let elements: HashMap<u64, PyObjectRef> = match iterable {
        OptionalArg::Missing => HashMap::new(),
        OptionalArg::Present(iterable) => {
            let mut elements = HashMap::new();
            let iterator = objiter::get_iter(vm, &iterable)?;
            while let Ok(v) = vm.call_method(&iterator, "__next__", vec![]) {
                insert_into_set(vm, &mut elements, &v)?;
            }
            elements
        }
    };

    create_set(vm, elements, cls.clone())
}

fn set_compare_inner(
    vm: &VirtualMachine,
    zelf: &PyObjectRef,
    other: &PyObjectRef,
    size_func: &Fn(usize, usize) -> bool,
    swap: bool,
) -> PyResult {
    validate_set_or_frozenset(vm, zelf.class())?;
    validate_set_or_frozenset(vm, other.class())?;

    let get_zelf = |swap: bool| -> &PyObjectRef {
        if swap {
            other
        } else {
            zelf
        }
    };
    let get_other = |swap: bool| -> &PyObjectRef {
        if swap {
            zelf
        } else {
            other
        }
    };

    let zelf_elements = get_elements(get_zelf(swap));
    let other_elements = get_elements(get_other(swap));
    if size_func(zelf_elements.len(), other_elements.len()) {
        return Ok(vm.new_bool(false));
    }
    for element in other_elements.iter() {
        match vm.call_method(get_zelf(swap), "__contains__", vec![element.1.clone()]) {
            Ok(value) => {
                if !objbool::get_value(&value) {
                    return Ok(vm.new_bool(false));
                }
            }
            Err(_) => return Err(vm.new_type_error("".to_string())),
        }
    }
    Ok(vm.new_bool(true))
}

enum SetCombineOperation {
    Intersection,
    Difference,
}

pub fn init(context: &PyContext) {
    let set_type = &context.set_type;

    let set_doc = "set() -> new empty set object\n\
                   set(iterable) -> new set object\n\n\
                   Build an unordered collection of unique elements.";

    extend_class!(context, set_type, {
        "__contains__" => context.new_rustfunc(PySetRef::contains),
        "__len__" => context.new_rustfunc(PySetRef::len),
        "__new__" => context.new_rustfunc(set_new),
        "__repr__" => context.new_rustfunc(PySetRef::repr),
        "__eq__" => context.new_rustfunc(PySetRef::eq),
        "__ge__" => context.new_rustfunc(PySetRef::ge),
        "__gt__" => context.new_rustfunc(PySetRef::gt),
        "__le__" => context.new_rustfunc(PySetRef::le),
        "__lt__" => context.new_rustfunc(PySetRef::lt),
        "issubset" => context.new_rustfunc(PySetRef::le),
        "issuperset" => context.new_rustfunc(PySetRef::ge),
        "union" => context.new_rustfunc(PySetRef::union),
        "__or__" => context.new_rustfunc(PySetRef::union),
        "intersection" => context.new_rustfunc(PySetRef::intersection),
        "__and__" => context.new_rustfunc(PySetRef::intersection),
        "difference" => context.new_rustfunc(PySetRef::difference),
        "__sub__" => context.new_rustfunc(PySetRef::difference),
        "symmetric_difference" => context.new_rustfunc(PySetRef::symmetric_difference),
        "__xor__" => context.new_rustfunc(PySetRef::symmetric_difference),
        "__doc__" => context.new_str(set_doc.to_string()),
        "add" => context.new_rustfunc(PySetRef::add),
        "remove" => context.new_rustfunc(PySetRef::remove),
        "discard" => context.new_rustfunc(PySetRef::discard),
        "clear" => context.new_rustfunc(PySetRef::clear),
        "copy" => context.new_rustfunc(PySetRef::copy),
        "pop" => context.new_rustfunc(PySetRef::pop),
        "update" => context.new_rustfunc(PySetRef::update),
        "__ior__" => context.new_rustfunc(PySetRef::ior),
        "intersection_update" => context.new_rustfunc(PySetRef::intersection_update),
        "__iand__" => context.new_rustfunc(PySetRef::iand),
        "difference_update" => context.new_rustfunc(PySetRef::difference_update),
        "__isub__" => context.new_rustfunc(PySetRef::isub),
        "symmetric_difference_update" => context.new_rustfunc(PySetRef::symmetric_difference_update),
        "__ixor__" => context.new_rustfunc(PySetRef::ixor),
        "__iter__" => context.new_rustfunc(PySetRef::iter)
    });

    let frozenset_type = &context.frozenset_type;

    let frozenset_doc = "frozenset() -> empty frozenset object\n\
                         frozenset(iterable) -> frozenset object\n\n\
                         Build an immutable unordered collection of unique elements.";

    extend_class!(context, frozenset_type, {
        "__new__" => context.new_rustfunc(set_new),
        "__eq__" => context.new_rustfunc(PyFrozenSetRef::eq),
        "__ge__" => context.new_rustfunc(PyFrozenSetRef::ge),
        "__gt__" => context.new_rustfunc(PyFrozenSetRef::gt),
        "__le__" => context.new_rustfunc(PyFrozenSetRef::le),
        "__lt__" => context.new_rustfunc(PyFrozenSetRef::lt),
        "issubset" => context.new_rustfunc(PyFrozenSetRef::le),
        "issuperset" => context.new_rustfunc(PyFrozenSetRef::ge),
        "union" => context.new_rustfunc(PyFrozenSetRef::union),
        "__or__" => context.new_rustfunc(PyFrozenSetRef::union),
        "intersection" => context.new_rustfunc(PyFrozenSetRef::intersection),
        "__and__" => context.new_rustfunc(PyFrozenSetRef::intersection),
        "difference" => context.new_rustfunc(PyFrozenSetRef::difference),
        "__sub__" => context.new_rustfunc(PyFrozenSetRef::difference),
        "symmetric_difference" => context.new_rustfunc(PyFrozenSetRef::symmetric_difference),
        "__xor__" => context.new_rustfunc(PyFrozenSetRef::symmetric_difference),
        "__contains__" => context.new_rustfunc(PyFrozenSetRef::contains),
        "__len__" => context.new_rustfunc(PyFrozenSetRef::len),
        "__doc__" => context.new_str(frozenset_doc.to_string()),
        "__repr__" => context.new_rustfunc(PyFrozenSetRef::repr),
        "copy" => context.new_rustfunc(PyFrozenSetRef::copy),
        "__iter__" => context.new_rustfunc(PyFrozenSetRef::iter)
    });
}
