use super::super::pyobject::{
    PyContext, PyFuncArgs, PyObject, PyObjectKind, PyObjectRef, PyResult, TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objstr;
use super::objtype;
use num_bigint::ToBigInt;
use std::cell::{Ref, RefMut};
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};

// This typedef abstracts the actual dict type used.
// pub type DictContentType = HashMap<usize, Vec<(PyObjectRef, PyObjectRef)>>;
// pub type DictContentType = HashMap<String, PyObjectRef>;
pub type DictContentType = HashMap<String, (PyObjectRef, PyObjectRef)>;

pub fn new(dict_type: PyObjectRef) -> PyObjectRef {
    PyObject::new(
        PyObjectKind::Dict {
            elements: HashMap::new(),
        },
        dict_type.clone(),
    )
}

pub fn get_elements<'a>(obj: &'a PyObjectRef) -> impl Deref<Target = DictContentType> + 'a {
    Ref::map(obj.borrow(), |py_obj| {
        if let PyObjectKind::Dict { ref elements } = py_obj.kind {
            elements
        } else {
            panic!("Cannot extract dict elements");
        }
    })
}

fn get_mut_elements<'a>(obj: &'a PyObjectRef) -> impl DerefMut<Target = DictContentType> + 'a {
    RefMut::map(obj.borrow_mut(), |py_obj| {
        if let PyObjectKind::Dict { ref mut elements } = py_obj.kind {
            elements
        } else {
            panic!("Cannot extract dict elements");
        }
    })
}

pub fn set_item(dict: &PyObjectRef, needle: &PyObjectRef, value: &PyObjectRef) {
    let mut elements = get_mut_elements(dict);
    set_item_in_content(&mut elements, needle, value);
}

pub fn set_item_in_content(
    elements: &mut DictContentType,
    needle: &PyObjectRef,
    value: &PyObjectRef,
) {
    // XXX: Currently, we only support String keys, so we have to unwrap the
    // PyObject (and ensure it is a String).

    let needle_str = objstr::get_value(needle);
    elements.insert(needle_str, (needle.clone(), value.clone()));
}

pub fn get_key_value_pairs(dict: &PyObjectRef) -> Vec<(PyObjectRef, PyObjectRef)> {
    let dict_elements = get_elements(dict);
    let mut pairs: Vec<(PyObjectRef, PyObjectRef)> = Vec::new();
    for (_str_key, pair) in dict_elements.iter() {
        let (key, obj) = pair;
        pairs.push((key.clone(), obj.clone()));
    }
    pairs
}

fn dict_new(_vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    Ok(new(args.args[0].clone()))
}

fn dict_len(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(dict_obj, Some(vm.ctx.dict_type()))]);
    let elements = get_elements(dict_obj);
    Ok(vm.ctx.new_int(elements.len().to_bigint().unwrap()))
}

fn dict_repr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(dict_obj, Some(vm.ctx.dict_type()))]);

    let elements = get_key_value_pairs(dict_obj);
    let mut str_parts = vec![];
    for (key, value) in elements {
        let s = vm.to_repr(&value)?;
        let key_str = objstr::get_value(&key);
        let value_str = objstr::get_value(&s);
        str_parts.push(format!("{}: {}", key_str, value_str));
    }

    let s = format!("{{{}}}", str_parts.join(", "));
    Ok(vm.new_str(s))
}

pub fn dict_contains(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [
            (dict, Some(vm.ctx.dict_type())),
            (needle, Some(vm.ctx.str_type()))
        ]
    );

    let needle = objstr::get_value(&needle);
    for element in get_elements(dict).iter() {
        if &needle == element.0 {
            return Ok(vm.new_bool(true));
        }
    }

    Ok(vm.new_bool(false))
}

fn dict_delitem(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [
            (dict, Some(vm.ctx.dict_type())),
            (needle, Some(vm.ctx.str_type()))
        ]
    );

    // What we are looking for:
    let needle = objstr::get_value(&needle);

    // Delete the item:
    let mut elements = get_mut_elements(dict);
    match elements.remove(&needle) {
        Some(_) => Ok(vm.get_none()),
        None => Err(vm.new_value_error(format!("Key not found: {}", needle))),
    }
}

/// When iterating over a dictionary, we iterate over the keys of it.
fn dict_iter(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(dict, Some(vm.ctx.dict_type()))]);

    let keys = get_elements(dict)
        .keys()
        .map(|k| vm.ctx.new_str(k.to_string()))
        .collect();
    let key_list = vm.ctx.new_list(keys);

    let iter_obj = PyObject::new(
        PyObjectKind::Iterator {
            position: 0,
            iterated_obj: key_list,
        },
        vm.ctx.iter_type(),
    );

    Ok(iter_obj)
}

fn dict_setitem(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [
            (dict, Some(vm.ctx.dict_type())),
            (needle, Some(vm.ctx.str_type())),
            (value, None)
        ]
    );

    set_item(dict, needle, value);

    Ok(vm.get_none())
}

fn dict_getitem(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [
            (dict, Some(vm.ctx.dict_type())),
            (needle, Some(vm.ctx.str_type()))
        ]
    );

    // What we are looking for:
    let needle = objstr::get_value(&needle);

    let elements = get_elements(dict);
    if elements.contains_key(&needle) {
        Ok(elements[&needle].1.clone())
    } else {
        Err(vm.new_value_error(format!("Key not found: {}", needle)))
    }
}

pub fn create_type(type_type: PyObjectRef, object_type: PyObjectRef, dict_type: PyObjectRef) {
    (*dict_type.borrow_mut()).kind = PyObjectKind::Class {
        name: String::from("dict"),
        dict: new(dict_type.clone()),
        mro: vec![object_type],
    };
    (*dict_type.borrow_mut()).typ = Some(type_type.clone());
}

pub fn init(context: &PyContext) {
    let ref dict_type = context.dict_type;
    context.set_attr(&dict_type, "__len__", context.new_rustfunc(dict_len));
    context.set_attr(
        &dict_type,
        "__contains__",
        context.new_rustfunc(dict_contains),
    );
    context.set_attr(
        &dict_type,
        "__delitem__",
        context.new_rustfunc(dict_delitem),
    );
    context.set_attr(
        &dict_type,
        "__getitem__",
        context.new_rustfunc(dict_getitem),
    );
    context.set_attr(&dict_type, "__iter__", context.new_rustfunc(dict_iter));
    context.set_attr(&dict_type, "__new__", context.new_rustfunc(dict_new));
    context.set_attr(&dict_type, "__repr__", context.new_rustfunc(dict_repr));
    context.set_attr(
        &dict_type,
        "__setitem__",
        context.new_rustfunc(dict_setitem),
    );
}
