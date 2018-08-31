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

fn dict_str(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(o, Some(vm.ctx.dict_type()))]);

    let elements = get_elements(o);
    let mut str_parts = vec![];
    for elem in elements {
        match vm.to_str(elem.1) {
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
    dict_type.set_attr("__new__", context.new_rustfunc(dict_new));
    dict_type.set_attr("__str__", context.new_rustfunc(dict_str));
}
