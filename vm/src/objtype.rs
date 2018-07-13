// use std::rc::Rc;
// use std::cell::RefCell;

/*
 * The magical type type
 */

use super::pyobject::{PyObject, PyObjectKind, PyObjectRef};

pub fn create_type() -> PyObjectRef {
    let typ = PyObject::default().into_ref();
    (*typ.borrow_mut()).kind = PyObjectKind::Type;
    (*typ.borrow_mut()).typ = Some(typ.clone());
    // typ.borrow_mut().dict.insert("__str__".to_string(), PyObject::new(PyObjectKind::RustFunction { function: str }));
    typ
}

