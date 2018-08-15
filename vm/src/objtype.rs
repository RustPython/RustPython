use super::pyobject::{PyFuncArgs, PyObject, PyObjectKind, PyObjectRef, PyResult};
use super::vm::VirtualMachine;
use std::collections::HashMap;

/*
 * The magical type type
 */

pub fn create_type() -> PyObjectRef {
    let typ = PyObject {
        kind: PyObjectKind::None,
        typ: None,
    }.into_ref();

    let dict = PyObject::new(
        PyObjectKind::Dict {
            elements: HashMap::new(),
        },
        typ.clone(),
    );
    (*typ.borrow_mut()).kind = PyObjectKind::Class {
        name: String::from("type"),
        dict: dict,
    };
    (*typ.borrow_mut()).typ = Some(typ.clone());
    typ
}

pub fn new(
    typ: PyObjectRef,
    name: String,
    _bases: Vec<PyObjectRef>,
    dict: PyObjectRef,
) -> PyObjectRef {
    PyObject::new(
        PyObjectKind::Class {
            name: name,
            dict: dict,
            // bases: bases
        },
        typ,
    )
}

fn noop(vm: &mut VirtualMachine, _args: PyFuncArgs) -> PyResult {
    Ok(vm.get_none())
}

pub fn create_object(type_type: PyObjectRef, function_type: PyObjectRef) -> PyObjectRef {
    let mut dict = HashMap::new();
    dict.insert(
        "__init__".to_string(),
        PyObject::new(
            PyObjectKind::RustFunction { function: noop },
            function_type.clone(),
        ),
    );
    new(
        type_type.clone(),
        String::from("object"),
        vec![],
        PyObject::new(PyObjectKind::Dict { elements: dict }, type_type.clone()),
    )
}
