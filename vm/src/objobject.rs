use super::objdict;
use super::objtype;
use super::pyobject::{
    AttributeProtocol, PyContext, PyFuncArgs, PyObject, PyObjectKind, PyObjectRef, PyResult,
};
use super::vm::VirtualMachine;

pub fn new_instance(vm: &mut VirtualMachine, mut args: PyFuncArgs) -> PyResult {
    // more or less __new__ operator
    let type_ref = args.shift();
    let dict = vm.new_dict();
    let obj = PyObject::new(PyObjectKind::Instance { dict: dict }, type_ref.clone());
    Ok(obj)
}

pub fn call(vm: &mut VirtualMachine, mut args: PyFuncArgs) -> PyResult {
    let instance = args.shift();
    let function = objtype::get_attribute(vm, instance, &String::from("__call__"))?;
    vm.invoke(function, args)
}

pub fn create_object(type_type: PyObjectRef, object_type: PyObjectRef, dict_type: PyObjectRef) {
    (*object_type.borrow_mut()).kind = PyObjectKind::Class {
        name: String::from("object"),
        dict: objdict::new(dict_type),
        mro: vec![],
    };
    (*object_type.borrow_mut()).typ = Some(type_type.clone());
}

pub fn init(context: &PyContext) {
    let ref object = context.object;
    object.set_attr("__new__", context.new_rustfunc(new_instance));
    object.set_attr("__dict__", context.new_member_descriptor(object_dict));
}

fn object_dict(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    match args.args[0].borrow().kind {
        PyObjectKind::Class { ref dict, .. } => Ok(dict.clone()),
        PyObjectKind::Instance { ref dict, .. } => Ok(dict.clone()),
        _ => Err(vm.new_type_error("TypeError: no dictionary.".to_string())),
    }
}
