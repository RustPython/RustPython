/*
 * Builtin set type with a sequence of unique items.
 */

use super::super::pyobject::{
    PyContext, PyFuncArgs, PyObject, PyObjectPayload, PyObjectRef, PyResult, TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objbool;
use super::objiter;
use super::objstr;
use super::objint;
use super::objtype;
use num_bigint::BigInt;
use std::collections::HashMap;

pub fn get_elements(obj: &PyObjectRef) -> HashMap<BigInt, PyObjectRef> {
    if let PyObjectPayload::Set { elements } = &obj.borrow().payload {
        elements.clone()
    } else {
        panic!("Cannot extract set elements from non-set");
    }
}

fn perform_action_with_hash(
    vm: &mut VirtualMachine,
    elements: &mut HashMap<BigInt, PyObjectRef>,
    item: &PyObjectRef,
    f: &Fn(
        &mut VirtualMachine,
        &mut HashMap<BigInt, PyObjectRef>,
        BigInt,
        &PyObjectRef,
    ) -> PyResult,
) -> PyResult {
    let hash: PyObjectRef = vm.call_method(item, "__hash__", vec![])?;

    let hash_value = objint::get_value(&hash);
    let key = hash_value.clone();
    f(vm, elements, key, item)
}

fn insert_into_set(
    vm: &mut VirtualMachine,
    elements: &mut HashMap<BigInt, PyObjectRef>,
    item: &PyObjectRef,
) -> PyResult {
    fn insert(
        vm: &mut VirtualMachine,
        elements: &mut HashMap<BigInt, PyObjectRef>,
        key: BigInt,
        value: &PyObjectRef,
    ) -> PyResult {
        elements.insert(key, value.clone());
        Ok(vm.get_none())
    }
    perform_action_with_hash(vm, elements, item, &insert)
}

fn set_add(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    trace!("set.add called with: {:?}", args);
    arg_check!(
        vm,
        args,
        required = [(s, Some(vm.ctx.set_type())), (item, None)]
    );
    let mut mut_obj = s.borrow_mut();

    match mut_obj.payload {
        PyObjectPayload::Set { ref mut elements } => insert_into_set(vm, elements, item),
        _ => Err(vm.new_type_error("set.add is called with no item".to_string())),
    }
}

fn set_remove(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    trace!("set.remove called with: {:?}", args);
    arg_check!(
        vm,
        args,
        required = [(s, Some(vm.ctx.set_type())), (item, None)]
    );
    let mut mut_obj = s.borrow_mut();

    match mut_obj.payload {
        PyObjectPayload::Set { ref mut elements } => {
            fn remove(
                vm: &mut VirtualMachine,
                elements: &mut HashMap<BigInt, PyObjectRef>,
                key: BigInt,
                value: &PyObjectRef,
            ) -> PyResult {
                match elements.remove(&key) {
                    None => {
                        let item = value.borrow();
                        Err(vm.new_key_error(item.str()))
                    }
                    Some(_) => Ok(vm.get_none()),
                }
            }
            perform_action_with_hash(vm, elements, item, &remove)
        }
        _ => Err(vm.new_type_error("set.remove is called with no item".to_string())),
    }
}

/* Create a new object of sub-type of set */
fn set_new(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(cls, None)],
        optional = [(iterable, None)]
    );

    if !objtype::issubclass(cls, &vm.ctx.set_type()) {
        return Err(vm.new_type_error(format!("{} is not a subtype of set", cls.borrow())));
    }

    let elements: HashMap<BigInt, PyObjectRef> = match iterable {
        None => HashMap::new(),
        Some(iterable) => {
            let mut elements = HashMap::new();
            let iterator = objiter::get_iter(vm, iterable)?;
            while let Ok(v) = vm.call_method(&iterator, "__next__", vec![]) {
                insert_into_set(vm, &mut elements, &v).unwrap();
            }
            elements
        }
    };

    Ok(PyObject::new(
        PyObjectPayload::Set { elements },
        cls.clone(),
    ))
}

fn set_len(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    trace!("set.len called with: {:?}", args);
    arg_check!(vm, args, required = [(s, Some(vm.ctx.set_type()))]);
    let elements = get_elements(s);
    Ok(vm.context().new_int(elements.len()))
}

fn set_repr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(o, Some(vm.ctx.set_type()))]);

    let elements = get_elements(o);
    let s = if elements.is_empty() {
        "set()".to_string()
    } else {
        let mut str_parts = vec![];
        for elem in elements.values() {
            let part = vm.to_repr(elem)?;
            str_parts.push(objstr::get_value(&part));
        }

        format!("{{{}}}", str_parts.join(", "))
    };
    Ok(vm.new_str(s))
}

pub fn set_contains(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(set, Some(vm.ctx.set_type())), (needle, None)]
    );
    for element in get_elements(set).iter() {
        match vm.call_method(needle, "__eq__", vec![element.1.clone()]) {
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

fn set_eq(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    return set_compare_inner(
        vm,
        args,
        &|zelf: usize, other: usize| -> bool { zelf != other },
        false,
    );
}

fn set_ge(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    return set_compare_inner(
        vm,
        args,
        &|zelf: usize, other: usize| -> bool { zelf < other },
        false,
    );
}

fn set_gt(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    return set_compare_inner(
        vm,
        args,
        &|zelf: usize, other: usize| -> bool { zelf <= other },
        false,
    );
}

fn set_le(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    return set_compare_inner(
        vm,
        args,
        &|zelf: usize, other: usize| -> bool { zelf < other },
        true,
    );
}

fn set_lt(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    return set_compare_inner(
        vm,
        args,
        &|zelf: usize, other: usize| -> bool { zelf <= other },
        true,
    );
}

fn set_compare_inner(
    vm: &mut VirtualMachine,
    args: PyFuncArgs,
    size_func: &Fn(usize, usize) -> bool,
    swap: bool,
) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [
            (zelf, Some(vm.ctx.set_type())),
            (other, Some(vm.ctx.set_type()))
        ]
    );

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
    let set_type = &context.set_type;

    let set_doc = "set() -> new empty set object\n\
                   set(iterable) -> new set object\n\n\
                   Build an unordered collection of unique elements.";

    context.set_attr(
        &set_type,
        "__contains__",
        context.new_rustfunc(set_contains),
    );
    context.set_attr(&set_type, "__len__", context.new_rustfunc(set_len));
    context.set_attr(&set_type, "__new__", context.new_rustfunc(set_new));
    context.set_attr(&set_type, "__repr__", context.new_rustfunc(set_repr));
    context.set_attr(&set_type, "__eq__", context.new_rustfunc(set_eq));
    context.set_attr(&set_type, "__ge__", context.new_rustfunc(set_ge));
    context.set_attr(&set_type, "__gt__", context.new_rustfunc(set_gt));
    context.set_attr(&set_type, "__le__", context.new_rustfunc(set_le));
    context.set_attr(&set_type, "__lt__", context.new_rustfunc(set_lt));
    context.set_attr(&set_type, "issubset", context.new_rustfunc(set_le));
    context.set_attr(&set_type, "issuperset", context.new_rustfunc(set_ge));
    context.set_attr(&set_type, "__doc__", context.new_str(set_doc.to_string()));
    context.set_attr(&set_type, "add", context.new_rustfunc(set_add));
    context.set_attr(&set_type, "remove", context.new_rustfunc(set_remove));

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
