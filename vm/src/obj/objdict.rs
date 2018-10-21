use super::super::pyobject::{
    AttributeProtocol, PyContext, PyFuncArgs, PyObject, PyObjectKind, PyObjectRef, PyResult,
    TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objstr;
use super::objtype;
use std::collections::HashMap;

pub fn _set_item(
    vm: &mut VirtualMachine,
    _d: PyObjectRef,
    _idx: PyObjectRef,
    _obj: PyObjectRef,
) -> PyResult {
    // TODO: Implement objdict::set_item
    Ok(vm.get_none())
}

pub fn new(dict_type: PyObjectRef) -> PyObjectRef {
    PyObject::new(
        PyObjectKind::Dict {
            elements: HashMap::new(),
        },
        dict_type.clone(),
    )
}

pub fn get_elements(obj: &PyObjectRef) -> HashMap<String, PyObjectRef> {
    if let PyObjectKind::Dict { elements } = &obj.borrow().kind {
        elements.clone()
    } else {
        panic!("Cannot extract dict elements");
    }
}

fn dict_new(_vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    Ok(new(args.args[0].clone()))
}

fn dict_len(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(o, Some(vm.ctx.dict_type()))]);
    let elements = get_elements(o);
    Ok(vm.ctx.new_int(elements.len() as i32))
}

fn dict_repr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(o, Some(vm.ctx.dict_type()))]);

    let elements = get_elements(o);
    let mut str_parts = vec![];
    for elem in elements {
        match vm.to_repr(elem.1) {
            Ok(s) => {
                let value_str = objstr::get_value(&s);
                str_parts.push(format!("{}: {}", elem.0, value_str));
            }
            Err(err) => return Err(err),
        }
    }

    let s = format!("{{ {} }}", str_parts.join(", "));
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

pub fn dict_delitem(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
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
    let mut dict_obj = dict.borrow_mut();
    if let PyObjectKind::Dict { ref mut elements } = dict_obj.kind {
        match elements.remove(&needle) {
            Some(_) => Ok(vm.get_none()),
            None => Err(vm.new_value_error(format!("Key not found: {}", needle))),
        }
    } else {
        panic!("Cannot extract dict elements");
    }
}

pub fn dict_getitem(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
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
        Ok(elements[&needle].clone())
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
    dict_type.set_attr("__len__", context.new_rustfunc(dict_len));
    dict_type.set_attr("__contains__", context.new_rustfunc(dict_contains));
    dict_type.set_attr("__delitem__", context.new_rustfunc(dict_delitem));
    dict_type.set_attr("__getitem__", context.new_rustfunc(dict_getitem));
    dict_type.set_attr("__new__", context.new_rustfunc(dict_new));
    dict_type.set_attr("__repr__", context.new_rustfunc(dict_repr));
}
