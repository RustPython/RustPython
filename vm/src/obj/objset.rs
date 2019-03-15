/*
 * Builtin set type with a sequence of unique items.
 */

use std::cell::{Cell, RefCell};
use std::collections::{hash_map::DefaultHasher, HashMap};
use std::fmt;
use std::hash::{Hash, Hasher};

use super::objbool;
use super::objint;
use super::objiter;
use super::objstr;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{
    OptionalArg, PyContext, PyFuncArgs, PyImmutableClass, PyIteratorValue, PyObject, PyObjectRef,
    PyRef, PyResult, PyValue, TypeProtocol,
};
use crate::vm::{ReprGuard, VirtualMachine};

#[derive(Clone, Default)]
pub struct PySet {
    elements: RefCell<HashMap<u64, PyObjectRef>>,
}

pub type PySetRef = PyRef<PySet>;

impl fmt::Debug for PySet {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // TODO: implement more detailed, non-recursive Debug formatter
        f.write_str("set")
    }
}

impl PyImmutableClass for PySet {
    fn create_type(ctx: &PyContext) -> PyClassRef {
        let cls = py_class!(ctx, "set", ctx.object(), {
            "__new__" => ctx.new_rustfunc(set_new),
            "__doc__" => ctx.new_str(
                "set() -> new empty set object\n\
                 set(iterable) -> new set object\n\n\
                 Build an unordered collection of unique elements.".to_string()),

            "__contains__" => ctx.new_rustfunc(set_contains),
            "__len__" => ctx.new_rustfunc(set_len),
            "__iter__" => ctx.new_rustfunc(set_iter),

            "issubset" => ctx.new_rustfunc(set_le),
            "issuperset" => ctx.new_rustfunc(set_ge),

            "add" => ctx.new_rustfunc(set_add),
            "remove" => ctx.new_rustfunc(set_remove),
            "discard" => ctx.new_rustfunc(set_discard),
            "clear" => ctx.new_rustfunc(set_clear),
            "copy" => ctx.new_rustfunc(set_copy),
            "pop" => ctx.new_rustfunc(set_pop),

            "union" => ctx.new_rustfunc(set_union),
            "__or__" => ctx.new_rustfunc(set_union),
            "update" => ctx.new_rustfunc(set_update),
            "__ior__" => ctx.new_rustfunc(set_ior),

            "intersection" => ctx.new_rustfunc(set_intersection),
            "__and__" => ctx.new_rustfunc(set_intersection),
            "intersection_update" => ctx.new_rustfunc(set_intersection_update),
            "__iand__" => ctx.new_rustfunc(set_iand),

            "difference" => ctx.new_rustfunc(set_difference),
            "__sub__" => ctx.new_rustfunc(set_difference),
            "difference_update" => ctx.new_rustfunc(set_difference_update),
            "__isub__" => ctx.new_rustfunc(set_isub),

            "symmetric_difference" => ctx.new_rustfunc(set_symmetric_difference),
            "__xor__" => ctx.new_rustfunc(set_symmetric_difference),
            "symmetric_difference_update" => ctx.new_rustfunc(set_symmetric_difference_update),
            "__ixor__" => ctx.new_rustfunc(set_ixor),

            "__eq__" => ctx.new_rustfunc(set_eq),
            "__ge__" => ctx.new_rustfunc(set_ge),
            "__gt__" => ctx.new_rustfunc(set_gt),
            "__le__" => ctx.new_rustfunc(set_le),
            "__lt__" => ctx.new_rustfunc(set_lt),

            "__repr__" => ctx.new_rustfunc(set_repr),
        });
        // TODO fix py_class so it returns a PyClassRef
        unsafe { PyRef::from_object_unchecked(cls) }
    }
}

fn get_elements(obj: &PyObjectRef) -> HashMap<u64, PyObjectRef> {
    obj.payload::<PySet>().unwrap().elements.borrow().clone()
}

fn perform_action_with_hash(
    vm: &mut VirtualMachine,
    elements: &mut HashMap<u64, PyObjectRef>,
    item: &PyObjectRef,
    f: &Fn(&mut VirtualMachine, &mut HashMap<u64, PyObjectRef>, u64, &PyObjectRef) -> PyResult,
) -> PyResult {
    let hash: PyObjectRef = vm.call_method(item, "__hash__", vec![])?;

    let hash_value = objint::get_value(&hash);
    let mut hasher = DefaultHasher::new();
    hash_value.hash(&mut hasher);
    let key = hasher.finish();
    f(vm, elements, key, item)
}

fn insert_into_set(
    vm: &mut VirtualMachine,
    elements: &mut HashMap<u64, PyObjectRef>,
    item: &PyObjectRef,
) -> PyResult {
    fn insert(
        vm: &mut VirtualMachine,
        elements: &mut HashMap<u64, PyObjectRef>,
        key: u64,
        value: &PyObjectRef,
    ) -> PyResult {
        elements.insert(key, value.clone());
        Ok(vm.get_none())
    }
    perform_action_with_hash(vm, elements, item, &insert)
}

fn set_add(set: PySetRef, item: PyObjectRef, vm: &mut VirtualMachine) -> PyResult {
    insert_into_set(vm, &mut set.elements.borrow_mut(), &item)
}

fn set_remove(set: PySetRef, item: PyObjectRef, vm: &mut VirtualMachine) -> PyResult {
    fn remove(
        vm: &mut VirtualMachine,
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
    perform_action_with_hash(vm, &mut set.elements.borrow_mut(), &item, &remove)
}

fn set_discard(set: PySetRef, item: PyObjectRef, vm: &mut VirtualMachine) -> PyResult {
    fn discard(
        vm: &mut VirtualMachine,
        elements: &mut HashMap<u64, PyObjectRef>,
        key: u64,
        _value: &PyObjectRef,
    ) -> PyResult {
        elements.remove(&key);
        Ok(vm.get_none())
    }
    perform_action_with_hash(vm, &mut set.elements.borrow_mut(), &item, &discard)
}

fn set_clear(set: PySetRef, _vm: &mut VirtualMachine) -> () {
    set.elements.borrow_mut().clear();
}

/* Create a new object of sub-type of set */
fn set_new(
    cls: PyClassRef,
    iterable: OptionalArg<PyObjectRef>,
    vm: &mut VirtualMachine,
) -> PyResult<PySetRef> {
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

    PySet {
        elements: RefCell::new(elements),
    }
    .into_ref_with_type(vm, cls)
}

fn set_len(set: PySetRef, _vm: &mut VirtualMachine) -> usize {
    set.elements.borrow().len()
}

fn set_copy(set: PySetRef, vm: &mut VirtualMachine) -> PySetRef {
    (*set)
        .clone()
        .into_ref_with_type(vm, set.typ())
        .expect("Can create new copy with same type")
}

fn set_repr(set: PySetRef, vm: &mut VirtualMachine) -> PyResult<String> {
    let elements = set.elements.borrow();
    Ok(if elements.is_empty() {
        "set()".to_string()
    } else if let Some(_guard) = ReprGuard::enter(set.as_object()) {
        let mut str_parts = vec![];
        for elem in elements.values() {
            let part = vm.to_repr(elem)?;
            str_parts.push(objstr::get_value(&part));
        }

        format!("{{{}}}", str_parts.join(", "))
    } else {
        "set(...)".to_string()
    })
}

pub fn set_contains(set: PySetRef, needle: PyObjectRef, vm: &mut VirtualMachine) -> PyResult<bool> {
    for element in set.elements.borrow().iter() {
        match vm._eq(needle.clone(), element.1.clone()) {
            Ok(value) => {
                if objbool::get_value(&value) {
                    return Ok(true);
                }
            }
            Err(_) => return Err(vm.new_type_error("".to_string())),
        }
    }

    Ok(false)
}

fn set_eq(zelf: PySetRef, other: PySetRef, vm: &mut VirtualMachine) -> PyResult<bool> {
    set_compare_inner(
        vm,
        zelf,
        other,
        &|zelf: usize, other: usize| -> bool { zelf != other },
        false,
    )
}

fn set_ge(zelf: PySetRef, other: PySetRef, vm: &mut VirtualMachine) -> PyResult<bool> {
    set_compare_inner(
        vm,
        zelf,
        other,
        &|zelf: usize, other: usize| -> bool { zelf < other },
        false,
    )
}

fn set_gt(zelf: PySetRef, other: PySetRef, vm: &mut VirtualMachine) -> PyResult<bool> {
    set_compare_inner(
        vm,
        zelf,
        other,
        &|zelf: usize, other: usize| -> bool { zelf <= other },
        false,
    )
}

fn set_le(zelf: PySetRef, other: PySetRef, vm: &mut VirtualMachine) -> PyResult<bool> {
    set_compare_inner(
        vm,
        zelf,
        other,
        &|zelf: usize, other: usize| -> bool { zelf < other },
        true,
    )
}

fn set_lt(zelf: PySetRef, other: PySetRef, vm: &mut VirtualMachine) -> PyResult<bool> {
    set_compare_inner(
        vm,
        zelf,
        other,
        &|zelf: usize, other: usize| -> bool { zelf <= other },
        true,
    )
}

fn set_compare_inner(
    vm: &mut VirtualMachine,
    zelf: PySetRef,
    other: PySetRef,
    size_func: &Fn(usize, usize) -> bool,
    swap: bool,
) -> PyResult<bool> {
    let get_zelf = |swap: bool| -> &PySetRef {
        if swap {
            &other
        } else {
            &zelf
        }
    };
    let get_other = |swap: bool| -> &PySetRef {
        if swap {
            &zelf
        } else {
            &other
        }
    };

    let zelf_elements = get_zelf(swap).elements.borrow();
    let other_elements = get_other(swap).elements.borrow();
    if size_func(zelf_elements.len(), other_elements.len()) {
        return Ok(false);
    }
    for element in other_elements.iter() {
        match vm.call_method(
            get_zelf(swap).as_object(),
            "__contains__",
            vec![element.1.clone()],
        ) {
            Ok(value) => {
                if !objbool::get_value(&value) {
                    return Ok(false);
                }
            }
            Err(_) => return Err(vm.new_type_error("".to_string())),
        }
    }
    Ok(true)
}

fn set_union(zelf: PySetRef, other: PySetRef, _vm: &mut VirtualMachine) -> PySet {
    let mut elements = zelf.elements.borrow().clone();
    elements.extend(other.elements.borrow().clone());

    PySet {
        elements: RefCell::new(elements),
    }
}

fn set_intersection(
    zelf: PySetRef,
    other: PySetRef,
    vm: &mut VirtualMachine,
) -> PyResult<PySetRef> {
    set_combine_inner(zelf, other, vm, SetCombineOperation::Intersection)
}

fn set_difference(zelf: PySetRef, other: PySetRef, vm: &mut VirtualMachine) -> PyResult<PySetRef> {
    set_combine_inner(zelf, other, vm, SetCombineOperation::Difference)
}

fn set_symmetric_difference(
    zelf: PySetRef,
    other: PySetRef,
    vm: &mut VirtualMachine,
) -> PyResult<PySetRef> {
    let mut elements = HashMap::new();

    for element in zelf.elements.borrow().iter() {
        let value = vm.call_method(other.as_object(), "__contains__", vec![element.1.clone()])?;
        if !objbool::get_value(&value) {
            elements.insert(element.0.clone(), element.1.clone());
        }
    }

    for element in other.elements.borrow().iter() {
        let value = vm.call_method(zelf.as_object(), "__contains__", vec![element.1.clone()])?;
        if !objbool::get_value(&value) {
            elements.insert(element.0.clone(), element.1.clone());
        }
    }

    Ok(PySet {
        elements: RefCell::new(elements),
    }
    .into_ref(vm))
}

enum SetCombineOperation {
    Intersection,
    Difference,
}

fn set_combine_inner(
    zelf: PySetRef,
    other: PySetRef,
    vm: &mut VirtualMachine,
    op: SetCombineOperation,
) -> PyResult<PySetRef> {
    let mut elements = HashMap::new();

    for element in zelf.elements.borrow().iter() {
        let value = vm.call_method(other.as_object(), "__contains__", vec![element.1.clone()])?;
        let should_add = match op {
            SetCombineOperation::Intersection => objbool::get_value(&value),
            SetCombineOperation::Difference => !objbool::get_value(&value),
        };
        if should_add {
            elements.insert(element.0.clone(), element.1.clone());
        }
    }

    Ok(PySet {
        elements: RefCell::new(elements),
    }
    .into_ref(vm))
}

fn set_pop(set: PySetRef, vm: &mut VirtualMachine) -> PyResult {
    let mut elements = set.elements.borrow_mut();
    match elements.clone().keys().next() {
        Some(key) => Ok(elements.remove(key).unwrap()),
        None => Err(vm.new_key_error("pop from an empty set".to_string())),
    }
}

fn set_update(set: PySetRef, iterable: PyObjectRef, vm: &mut VirtualMachine) -> PyResult<()> {
    set_ior(set, iterable, vm)?;
    Ok(())
}

fn set_ior(set: PySetRef, iterable: PyObjectRef, vm: &mut VirtualMachine) -> PyResult<PySetRef> {
    let iterator = objiter::get_iter(vm, &iterable)?;
    while let Ok(v) = vm.call_method(&iterator, "__next__", vec![]) {
        insert_into_set(vm, &mut set.elements.borrow_mut(), &v)?;
    }
    Ok(set.clone())
}

fn set_intersection_update(
    set: PySetRef,
    iterable: PyObjectRef,
    vm: &mut VirtualMachine,
) -> PyResult {
    set_combine_update_inner(set, iterable, vm, SetCombineOperation::Intersection)?;
    Ok(vm.get_none())
}

fn set_iand(set: PySetRef, iterable: PyObjectRef, vm: &mut VirtualMachine) -> PyResult {
    set_combine_update_inner(set, iterable, vm, SetCombineOperation::Intersection)
}

fn set_difference_update(
    set: PySetRef,
    iterable: PyObjectRef,
    vm: &mut VirtualMachine,
) -> PyResult {
    set_combine_update_inner(set, iterable, vm, SetCombineOperation::Difference)?;
    Ok(vm.get_none())
}

fn set_isub(set: PySetRef, iterable: PyObjectRef, vm: &mut VirtualMachine) -> PyResult {
    set_combine_update_inner(set, iterable, vm, SetCombineOperation::Difference)
}

fn set_combine_update_inner(
    set: PySetRef,
    iterable: PyObjectRef,
    vm: &mut VirtualMachine,
    op: SetCombineOperation,
) -> PyResult {
    {
        let mut elements = set.elements.borrow_mut();
        for element in elements.clone().iter() {
            let value = vm.call_method(&iterable, "__contains__", vec![element.1.clone()])?;
            let should_remove = match op {
                SetCombineOperation::Intersection => !objbool::get_value(&value),
                SetCombineOperation::Difference => objbool::get_value(&value),
            };
            if should_remove {
                elements.remove(&element.0.clone());
            }
        }
    }
    Ok(set.into_object())
}

fn set_symmetric_difference_update(
    set: PySetRef,
    iterable: PyObjectRef,
    vm: &mut VirtualMachine,
) -> PyResult {
    set_ixor(set, iterable, vm)?;
    Ok(vm.get_none())
}

fn set_ixor(set: PySetRef, iterable: PyObjectRef, vm: &mut VirtualMachine) -> PyResult {
    let elements_original = set.elements.borrow().clone();
    let iterator = objiter::get_iter(vm, &iterable)?;
    while let Ok(v) = vm.call_method(&iterator, "__next__", vec![]) {
        insert_into_set(vm, &mut set.elements.borrow_mut(), &v)?;
    }
    for element in elements_original.iter() {
        let value = vm.call_method(&iterable, "__contains__", vec![element.1.clone()])?;
        if objbool::get_value(&value) {
            set.elements.borrow_mut().remove(&element.0.clone());
        }
    }
    Ok(set.into_object())
}

fn set_iter(set: PySetRef, vm: &mut VirtualMachine) -> PyResult {
    let items = set.elements.borrow().values().cloned().collect();
    let set_list = vm.ctx.new_list(items);
    let iter_obj = PyObject::new(
        PyIteratorValue {
            position: Cell::new(0),
            iterated_obj: set_list,
        },
        vm.ctx.iter_type(),
    );

    Ok(iter_obj)
}

fn frozenset_repr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(o, Some(vm.ctx.frozenset_type()))]);

    let elements = get_elements(o);
    let s = if elements.is_empty() {
        "frozenset()".to_string()
    } else {
        let mut str_parts = vec![];
        for elem in elements.values() {
            let part = vm.to_repr(elem)?;
            str_parts.push(objstr::get_value(&part));
        }

        format!("frozenset({{{}}})", str_parts.join(", "))
    };
    Ok(vm.new_str(s))
}

pub fn init(context: &PyContext) {
    let frozenset_type = &context.frozenset_type;

    let frozenset_doc = "frozenset() -> empty frozenset object\n\
                         frozenset(iterable) -> frozenset object\n\n\
                         Build an immutable unordered collection of unique elements.";

    context.set_attr(
        &frozenset_type,
        "__contains__",
        context.new_rustfunc(set_contains),
    );
    context.set_attr(&frozenset_type, "__len__", context.new_rustfunc(set_len));
    context.set_attr(
        &frozenset_type,
        "__doc__",
        context.new_str(frozenset_doc.to_string()),
    );
    context.set_attr(
        &frozenset_type,
        "__repr__",
        context.new_rustfunc(frozenset_repr),
    );
}
