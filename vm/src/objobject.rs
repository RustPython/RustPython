use super::objtype;
use super::pyobject::{
    AttributeProtocol, PyContext, PyFuncArgs, PyObject, PyObjectKind, PyObjectRef, PyResult,
};
use super::vm::VirtualMachine;
use std::collections::HashMap;

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

fn noop(vm: &mut VirtualMachine, _args: PyFuncArgs) -> PyResult {
    Ok(vm.get_none())
}

pub fn create_object(type_type: PyObjectRef) -> PyObjectRef {
    let dict = PyObject::new(
        PyObjectKind::Dict {
            elements: HashMap::new(),
        },
        type_type.clone(),
    );
    objtype::new(type_type.clone(), "object", vec![], dict).unwrap()
}

pub fn init(context: &PyContext) {
    let ref object = context.object;
    object.set_attr("__new__", context.new_rustfunc(new_instance));
    object.set_attr("__init__", context.new_rustfunc(noop));
}
