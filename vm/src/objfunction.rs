use super::pyobject::{PyFuncArgs, PyObject, PyObjectKind, PyObjectRef, PyResult};
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
