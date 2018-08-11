use std::collections::HashMap;

/*
 * The magical type type
 */

use super::pyobject::{PyObject, PyObjectKind, PyObjectRef};

pub fn create_type() -> PyObjectRef {
    let typ = PyObject::default().into_ref();
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
