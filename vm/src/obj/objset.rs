/*
 * Builtin set type with a sequence of unique items.
 */

use std::cell::{Cell, RefCell};
use std::collections::{hash_map::DefaultHasher, HashMap};
use std::fmt;
use std::hash::{Hash, Hasher};

use crate::function::{OptionalArg, PyFuncArgs};
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

fn set_add(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    trace!("set.add called with: {:?}", args);
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.set_type())), (item, None)]
    );
    match zelf.payload::<PySet>() {
        Some(set) => insert_into_set(vm, &mut set.elements.borrow_mut(), item),
        _ => Err(vm.new_type_error("set.add is called with no item".to_string())),
    }
}

fn set_remove(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    trace!("set.remove called with: {:?}", args);
    arg_check!(
        vm,
        args,
        required = [(s, Some(vm.ctx.set_type())), (item, None)]
    );
    match s.payload::<PySet>() {
        Some(set) => {
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
            perform_action_with_hash(vm, &mut set.elements.borrow_mut(), item, &remove)
        }
        _ => Err(vm.new_type_error("set.remove is called with no item".to_string())),
    }
}

fn set_discard(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    trace!("set.discard called with: {:?}", args);
    arg_check!(
        vm,
        args,
        required = [(s, Some(vm.ctx.set_type())), (item, None)]
    );
    match s.payload::<PySet>() {
        Some(set) => {
            fn discard(
                vm: &VirtualMachine,
                elements: &mut HashMap<u64, PyObjectRef>,
                key: u64,
                _value: &PyObjectRef,
            ) -> PyResult {
                elements.remove(&key);
                Ok(vm.get_none())
            }
            perform_action_with_hash(vm, &mut set.elements.borrow_mut(), item, &discard)
        }
        None => Err(vm.new_type_error("set.discard is called with no item".to_string())),
    }
}

fn set_clear(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    trace!("set.clear called");
    arg_check!(vm, args, required = [(s, Some(vm.ctx.set_type()))]);
    match s.payload::<PySet>() {
        Some(set) => {
            set.elements.borrow_mut().clear();
            Ok(vm.get_none())
        }
        None => Err(vm.new_type_error("".to_string())),
    }
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

fn set_repr(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(o, Some(vm.ctx.set_type()))]);

    let elements = get_elements(o);
    let s = if elements.is_empty() {
        "set()".to_string()
    } else if let Some(_guard) = ReprGuard::enter(o) {
        let mut str_parts = vec![];
        for elem in elements.values() {
            let part = vm.to_repr(elem)?;
            str_parts.push(part.value.clone());
        }

        format!("{{{}}}", str_parts.join(", "))
    } else {
        "set(...)".to_string()
    };
    Ok(vm.new_str(s))
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

fn set_pop(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(s, Some(vm.ctx.set_type()))]);

    match s.payload::<PySet>() {
        Some(set) => {
            let mut elements = set.elements.borrow_mut();
            match elements.clone().keys().next() {
                Some(key) => Ok(elements.remove(key).unwrap()),
                None => Err(vm.new_key_error("pop from an empty set".to_string())),
            }
        }
        _ => Err(vm.new_type_error("".to_string())),
    }
}

fn set_update(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    set_ior(vm, args)?;
    Ok(vm.get_none())
}

fn set_ior(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.set_type())), (iterable, None)]
    );

    match zelf.payload::<PySet>() {
        Some(set) => {
            let iterator = objiter::get_iter(vm, iterable)?;
            while let Ok(v) = vm.call_method(&iterator, "__next__", vec![]) {
                insert_into_set(vm, &mut set.elements.borrow_mut(), &v)?;
            }
        }
        _ => return Err(vm.new_type_error("set.update is called with no other".to_string())),
    }
    Ok(zelf.clone())
}

fn set_intersection_update(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    set_combine_update_inner(vm, args, SetCombineOperation::Intersection)?;
    Ok(vm.get_none())
}

fn set_iand(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    set_combine_update_inner(vm, args, SetCombineOperation::Intersection)
}

fn set_difference_update(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    set_combine_update_inner(vm, args, SetCombineOperation::Difference)?;
    Ok(vm.get_none())
}

fn set_isub(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    set_combine_update_inner(vm, args, SetCombineOperation::Difference)
}

fn set_combine_update_inner(
    vm: &VirtualMachine,
    args: PyFuncArgs,
    op: SetCombineOperation,
) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.set_type())), (iterable, None)]
    );

    match zelf.payload::<PySet>() {
        Some(set) => {
            let mut elements = set.elements.borrow_mut();
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
        }
        _ => return Err(vm.new_type_error("".to_string())),
    }
    Ok(zelf.clone())
}

fn set_symmetric_difference_update(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    set_ixor(vm, args)?;
    Ok(vm.get_none())
}

fn set_ixor(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.set_type())), (iterable, None)]
    );

    match zelf.payload::<PySet>() {
        Some(set) => {
            let elements_original = set.elements.borrow().clone();
            let iterator = objiter::get_iter(vm, iterable)?;
            while let Ok(v) = vm.call_method(&iterator, "__next__", vec![]) {
                insert_into_set(vm, &mut set.elements.borrow_mut(), &v)?;
            }
            for element in elements_original.iter() {
                let value = vm.call_method(iterable, "__contains__", vec![element.1.clone()])?;
                if objbool::get_value(&value) {
                    set.elements.borrow_mut().remove(&element.0.clone());
                }
            }
        }
        _ => return Err(vm.new_type_error("".to_string())),
    }

    Ok(zelf.clone())
}

fn set_iter(zelf: PySetRef, vm: &VirtualMachine) -> PyIteratorValue {
    let items = zelf.elements.borrow().values().cloned().collect();
    let set_list = vm.ctx.new_list(items);
    PyIteratorValue {
        position: Cell::new(0),
        iterated_obj: set_list,
    }
}

fn frozenset_repr(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(o, Some(vm.ctx.frozenset_type()))]);

    let elements = get_elements(o);
    let s = if elements.is_empty() {
        "frozenset()".to_string()
    } else {
        let mut str_parts = vec![];
        for elem in elements.values() {
            let part = vm.to_repr(elem)?;
            str_parts.push(part.value.clone());
        }

        format!("frozenset({{{}}})", str_parts.join(", "))
    };
    Ok(vm.new_str(s))
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
        "__repr__" => context.new_rustfunc(set_repr),
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
        "add" => context.new_rustfunc(set_add),
        "remove" => context.new_rustfunc(set_remove),
        "discard" => context.new_rustfunc(set_discard),
        "clear" => context.new_rustfunc(set_clear),
        "copy" => context.new_rustfunc(PySetRef::copy),
        "pop" => context.new_rustfunc(set_pop),
        "update" => context.new_rustfunc(set_update),
        "__ior__" => context.new_rustfunc(set_ior),
        "intersection_update" => context.new_rustfunc(set_intersection_update),
        "__iand__" => context.new_rustfunc(set_iand),
        "difference_update" => context.new_rustfunc(set_difference_update),
        "__isub__" => context.new_rustfunc(set_isub),
        "symmetric_difference_update" => context.new_rustfunc(set_symmetric_difference_update),
        "__ixor__" => context.new_rustfunc(set_ixor),
        "__iter__" => context.new_rustfunc(set_iter)
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
        "__repr__" => context.new_rustfunc(frozenset_repr),
        "copy" => context.new_rustfunc(PyFrozenSetRef::copy)
    });
}
