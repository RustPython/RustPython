use super::pyobject::{PyFuncArgs, PyObject, PyObjectKind, PyObjectRef};
use super::vm::VirtualMachine;
use std::collections::HashMap;

fn str(vm: &mut VirtualMachine, _args: PyFuncArgs) -> Result<PyObjectRef, PyObjectRef> {
    // TODO: Implement objint::str
    Ok(vm.new_str("todo".to_string()))
}

/*
fn set_attr(a: &mut PyObjectRef, name: String, b: PyObjectRef) {
    a.borrow().dict.insert(name, b);
}
*/

pub fn create_type(type_type: PyObjectRef) -> PyObjectRef {
    let mut dict = HashMap::new();
    dict.insert(
        "__str__".to_string(),
        PyObject::new(
            PyObjectKind::RustFunction { function: str },
            type_type.clone(),
        ),
    );
    let typ = PyObject::new(
        PyObjectKind::Class {
            name: "int".to_string(),
            dict: PyObject::new(PyObjectKind::Dict { elements: dict }, type_type.clone()),
        },
        type_type.clone(),
    );
    typ
}
