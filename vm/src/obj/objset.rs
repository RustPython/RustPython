/*
 * Builtin set type with a sequence of unique items.
 */

use super::super::pyobject::{
    IdProtocol, PyContext, PyFuncArgs, PyObject, PyObjectPayload, PyObjectRef, PyResult,
    TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objbool;
use super::objiter;
use super::objstr;
use super::objtype;
use num_bigint::ToBigInt;
use std::collections::HashMap;
use num_bigint::BigInt;

pub fn get_elements(obj: &PyObjectRef) -> HashMap<BigInt, PyObjectRef> {
    if let PyObjectPayload::Set { elements } = &obj.borrow().payload {
        elements.clone()
    } else {
        panic!("Cannot extract set elements from non-set");
    }
}

pub fn sequence_to_hashmap(iterable: &Vec<PyObjectRef>) -> HashMap<usize, PyObjectRef> {
    let mut elements = HashMap::new();
    for item in iterable {
        let key = item.get_id();
        elements.insert(key, item.clone());
    }
    elements
}

fn perform_action_with_hash(vm: &mut VirtualMachine, elements: &mut HashMap<BigInt, PyObjectRef>, item: &PyObjectRef,
                            f: &Fn(&mut VirtualMachine, &mut HashMap<BigInt, PyObjectRef>, BigInt, &PyObjectRef) -> PyResult) -> PyResult {
    let hash_result: PyResult = vm.call_method(item, "__hash__", vec![]);
    match hash_result {
        Ok(hash_object) => {
            let hash = hash_object.borrow();
            match hash.payload {
                PyObjectPayload::Integer { ref value } => {
                    let key = value.clone();
                    f(vm, elements, key, item)
                },
                _ => { Err(vm.new_type_error(format!("__hash__ method should return an integer"))) }
            }
        },
        Err(error) => Err(error),
    }
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
        PyObjectPayload::Set {ref mut elements} => {
            fn insert_into_set(vm: &mut VirtualMachine, elements: &mut HashMap<BigInt, PyObjectRef>,
                               key: BigInt, value: &PyObjectRef) -> PyResult {
                elements.insert(key, value.clone());
                Ok(vm.get_none())
            }
            perform_action_with_hash(vm, elements, item, &insert_into_set)},
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
        PyObjectPayload::Set {ref mut elements} => {
            fn remove_from_set(vm: &mut VirtualMachine, elements: &mut HashMap<BigInt, PyObjectRef>,
                               key: BigInt, value: &PyObjectRef) -> PyResult {
                match elements.remove(&key) {
                    None => {
                        let item = value.borrow();
                        Err(vm.new_key_error(item.str()))
                    },
                    Some(_) => {Ok(vm.get_none())},
                }
            }
            perform_action_with_hash(vm, elements, item, &remove_from_set)},
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

//    let elements = match iterable {
//        None => HashMap::new(),
//        Some(iterable) => {
//            let mut elements = HashMap::new();
//            let iterator = objiter::get_iter(vm, iterable)?;
//            loop {
//                match vm.call_method(&iterator, "__next__", vec![]) {
//                    Ok(v) => {
//                        // TODO: should we use the hash function here?
//                        let key = v.get_id();
//                        elements.insert(key, v);
//                    }
//                    _ => break,
//                }
//            }
//            elements
//        }
//    };

    Ok(PyObject::new(
        PyObjectPayload::Set { elements: HashMap::new() },
        cls.clone(),
    ))
}

fn set_len(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    trace!("set.len called with: {:?}", args);
    arg_check!(vm, args, required = [(s, Some(vm.ctx.set_type()))]);
    let elements = get_elements(s);
    Ok(vm.context().new_int(elements.len().to_bigint().unwrap()))
}

fn set_repr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(o, Some(vm.ctx.set_type()))]);

    let elements = get_elements(o);
    let s = if elements.len() == 0 {
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

fn frozenset_repr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(o, Some(vm.ctx.frozenset_type()))]);

    let elements = get_elements(o);
    let s = if elements.len() == 0 {
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
    let ref set_type = context.set_type;
    context.set_attr(
        &set_type,
        "__contains__",
        context.new_rustfunc(set_contains),
    );
    context.set_attr(&set_type, "__len__", context.new_rustfunc(set_len));
    context.set_attr(&set_type, "__new__", context.new_rustfunc(set_new));
    context.set_attr(&set_type, "__repr__", context.new_rustfunc(set_repr));
    context.set_attr(&set_type, "add", context.new_rustfunc(set_add));
    context.set_attr(&set_type, "remove", context.new_rustfunc(set_remove));

    let ref frozenset_type = context.frozenset_type;
    context.set_attr(
        &frozenset_type,
        "__contains__",
        context.new_rustfunc(set_contains),
    );
    context.set_attr(&frozenset_type, "__len__", context.new_rustfunc(set_len));
    context.set_attr(
        &frozenset_type,
        "__repr__",
        context.new_rustfunc(frozenset_repr),
    );
}
