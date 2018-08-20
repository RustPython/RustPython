use super::objtype;
use super::pyobject::{
    AttributeProtocol, PyFuncArgs, PyObject, PyObjectKind, PyObjectRef, PyResult,
};
use super::vm::VirtualMachine;
use std::collections::HashMap;

pub fn create_type(type_type: PyObjectRef) -> PyObjectRef {
    let mut dict = HashMap::new();
    dict.insert(
        "__get__".to_string(),
        PyObject::new(
            PyObjectKind::RustFunction {
                function: bind_method,
            },
            type_type.clone(),
        ),
    );
    let typ = PyObject::new(
        PyObjectKind::Class {
            name: "function".to_string(),
            dict: PyObject::new(PyObjectKind::Dict { elements: dict }, type_type.clone()),
            mro: vec![],
        },
        type_type.clone(),
    );
    typ
}

pub fn create_bound_method_type(type_type: PyObjectRef) -> PyObjectRef {
    let dict = HashMap::new();
    let typ = PyObject::new(
        PyObjectKind::Class {
            name: "method".to_string(),
            dict: PyObject::new(PyObjectKind::Dict { elements: dict }, type_type.clone()),
            mro: vec![],
        },
        type_type.clone(),
    );
    typ
}

fn bind_method(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    Ok(vm.new_bound_method(args.args[0].clone(), args.args[1].clone()))
}

pub fn create_member_descriptor_type(type_type: PyObjectRef, object: PyObjectRef) -> PyResult {
    let mut dict = HashMap::new();

    dict.insert(
        String::from("__get__"),
        PyObject::new(
            PyObjectKind::RustFunction {
                function: member_get,
            },
            type_type.clone(),
        ),
    );

    objtype::new(
        type_type.clone(),
        "member_descriptor",
        vec![object],
        PyObject::new(PyObjectKind::Dict { elements: dict }, type_type.clone()),
    )
}

fn member_get(vm: &mut VirtualMachine, mut args: PyFuncArgs) -> PyResult {
    let function = args.shift().get_attr(&String::from("function"));
    vm.invoke(function, args)
}
