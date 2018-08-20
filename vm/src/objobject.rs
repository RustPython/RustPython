use super::objtype;
use super::pyobject::{PyFuncArgs, PyObject, PyObjectKind, PyObjectRef, PyResult};
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

pub fn create_object(type_type: PyObjectRef, function_type: PyObjectRef) -> PyObjectRef {
    let mut dict = HashMap::new();
    dict.insert(
        "__new__".to_string(),
        PyObject::new(
            PyObjectKind::RustFunction {
                function: new_instance,
            },
            function_type.clone(),
        ),
    );
    dict.insert(
        "__init__".to_string(),
        PyObject::new(
            PyObjectKind::RustFunction { function: noop },
            function_type.clone(),
        ),
    );
    objtype::new(
        type_type.clone(),
        "object",
        vec![],
        PyObject::new(PyObjectKind::Dict { elements: dict }, type_type.clone()),
    ).unwrap()
}
