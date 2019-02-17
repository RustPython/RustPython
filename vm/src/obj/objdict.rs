use super::super::pyobject::{
    PyAttributes, PyContext, PyFuncArgs, PyObject, PyObjectPayload, PyObjectRef, PyResult,
    TypeProtocol,
};
use super::super::vm::{ReprGuard, VirtualMachine};
use super::objiter;
use super::objstr;
use super::objtype;
use std::cell::{Ref, RefCell, RefMut};
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};

// This typedef abstracts the actual dict type used.
// pub type DictContentType = HashMap<usize, Vec<(PyObjectRef, PyObjectRef)>>;
// pub type DictContentType = HashMap<String, PyObjectRef>;
pub type DictContentType = HashMap<String, (PyObjectRef, PyObjectRef)>;
// pub type DictContentType = HashMap<String, Vec<(PyObjectRef, PyObjectRef)>>;

pub fn new(dict_type: PyObjectRef) -> PyObjectRef {
    PyObject::new(
        PyObjectPayload::Dict {
            elements: HashMap::new(),
        },
        dict_type.clone(),
    )
}

pub fn get_elements<'a>(obj: &'a PyObjectRef) -> impl Deref<Target = DictContentType> + 'a {
    Ref::map(obj.borrow(), |py_obj| {
        if let PyObjectPayload::Dict { ref elements } = py_obj.payload {
            elements
        } else {
            panic!("Cannot extract dict elements");
        }
    })
}

fn get_mut_elements<'a>(obj: &'a PyObjectRef) -> impl DerefMut<Target = DictContentType> + 'a {
    RefMut::map(obj.borrow_mut(), |py_obj| {
        if let PyObjectPayload::Dict { ref mut elements } = py_obj.payload {
            elements
        } else {
            panic!("Cannot extract dict elements");
        }
    })
}

pub fn set_item(
    dict: &PyObjectRef,
    _vm: &mut VirtualMachine,
    needle: &PyObjectRef,
    value: &PyObjectRef,
) {
    // TODO: use vm to call eventual __hash__ and __eq__methods.
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

    // TODO: invoke __hash__ function here!
    let needle_str = objstr::get_value(needle);
    elements.insert(needle_str, (needle.clone(), value.clone()));
}

pub fn get_key_value_pairs(dict: &PyObjectRef) -> Vec<(PyObjectRef, PyObjectRef)> {
    let dict_elements = get_elements(dict);
    get_key_value_pairs_from_content(&dict_elements)
}

pub fn get_key_value_pairs_from_content(
    dict_content: &DictContentType,
) -> Vec<(PyObjectRef, PyObjectRef)> {
    let mut pairs: Vec<(PyObjectRef, PyObjectRef)> = Vec::new();
    for (_str_key, pair) in dict_content.iter() {
        let (key, obj) = pair;
        pairs.push((key.clone(), obj.clone()));
    }
    pairs
}

pub fn get_item(dict: &PyObjectRef, key: &PyObjectRef) -> Option<PyObjectRef> {
    let needle_str = objstr::get_value(key);
    get_key_str(dict, &needle_str)
}

// Special case for the case when requesting a str key from a dict:
pub fn get_key_str(dict: &PyObjectRef, key: &str) -> Option<PyObjectRef> {
    let elements = get_elements(dict);
    content_get_key_str(&elements, key)
}

/// Retrieve a key from dict contents:
pub fn content_get_key_str(elements: &DictContentType, key: &str) -> Option<PyObjectRef> {
    // TODO: let hash: usize = key;
    match elements.get(key) {
        Some(v) => Some(v.1.clone()),
        None => None,
    }
}

pub fn contains_key_str(dict: &PyObjectRef, key: &str) -> bool {
    let elements = get_elements(dict);
    content_contains_key_str(&elements, key)
}

pub fn content_contains_key_str(elements: &DictContentType, key: &str) -> bool {
    // TODO: let hash: usize = key;
    elements.get(key).is_some()
}

/// Take a python dictionary and convert it to attributes.
pub fn py_dict_to_attributes(dict: &PyObjectRef) -> PyAttributes {
    let mut attrs = PyAttributes::new();
    for (key, value) in get_key_value_pairs(dict) {
        let key = objstr::get_value(&key);
        attrs.insert(key, value);
    }
    attrs
}

pub fn attributes_to_py_dict(vm: &mut VirtualMachine, attributes: PyAttributes) -> PyResult {
    let dict = vm.ctx.new_dict();
    for (key, value) in attributes {
        let key = vm.ctx.new_str(key);
        set_item(&dict, vm, &key, &value);
    }
    Ok(dict)
}

// Python dict methods:

fn dict_new(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(_ty, Some(vm.ctx.type_type()))],
        optional = [(dict_obj, None)]
    );
    let dict = vm.ctx.new_dict();
    if let Some(dict_obj) = dict_obj {
        if objtype::isinstance(&dict_obj, &vm.ctx.dict_type()) {
            for (needle, value) in get_key_value_pairs(&dict_obj) {
                set_item(&dict, vm, &needle, &value);
            }
        } else {
            let iter = objiter::get_iter(vm, dict_obj)?;
            loop {
                fn err(vm: &mut VirtualMachine) -> PyObjectRef {
                    vm.new_type_error("Iterator must have exactly two elements".to_string())
                }
                let element = match objiter::get_next_object(vm, &iter)? {
                    Some(obj) => obj,
                    None => break,
                };
                let elem_iter = objiter::get_iter(vm, &element)?;
                let needle = objiter::get_next_object(vm, &elem_iter)?.ok_or_else(|| err(vm))?;
                let value = objiter::get_next_object(vm, &elem_iter)?.ok_or_else(|| err(vm))?;
                if objiter::get_next_object(vm, &elem_iter)?.is_some() {
                    return Err(err(vm));
                }
                set_item(&dict, vm, &needle, &value);
            }
        }
    }
    for (needle, value) in args.kwargs {
        let py_needle = vm.new_str(needle);
        set_item(&dict, vm, &py_needle, &value);
    }
    Ok(dict)
}

fn dict_len(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(dict_obj, Some(vm.ctx.dict_type()))]);
    let elements = get_elements(dict_obj);
    Ok(vm.ctx.new_int(elements.len()))
}

fn dict_repr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(dict_obj, Some(vm.ctx.dict_type()))]);

    let s = if let Some(_guard) = ReprGuard::enter(dict_obj) {
        let elements = get_key_value_pairs(dict_obj);
        let mut str_parts = vec![];
        for (key, value) in elements {
            let key_repr = vm.to_repr(&key)?;
            let value_repr = vm.to_repr(&value)?;
            let key_str = objstr::get_value(&key_repr);
            let value_str = objstr::get_value(&value_repr);
            str_parts.push(format!("{}: {}", key_str, value_str));
        }

        format!("{{{}}}", str_parts.join(", "))
    } else {
        "{...}".to_string()
    };
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
        PyObjectPayload::Iterator {
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

    set_item(dict, vm, needle, value);

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
    (*dict_type.borrow_mut()).payload = PyObjectPayload::Class {
        name: String::from("dict"),
        dict: RefCell::new(HashMap::new()),
        mro: vec![object_type],
    };
    (*dict_type.borrow_mut()).typ = Some(type_type.clone());
}

pub fn init(context: &PyContext) {
    let dict_type = &context.dict_type;
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
