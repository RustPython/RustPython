use super::pyobject::{PyFuncArgs, PyObject, PyObjectKind, PyObjectRef, PyResult, TypeProtocol};
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
        },
        type_type.clone(),
    );
    typ
}

fn bind_method(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    unimplemented!("not yet!");
}
