
/*
 * The magical type type
 */

use super::pyobject::{PyObject, PyObjectRef, PyObjectKind};

pub fn create_type() -> PyObjectRef {
    let mut typ = PyObject::new(PyObjectKind::Type);
    // typ.borrow_mut().dict.insert("__str__".to_string(), PyObject::new(PyObjectKind::RustFunction { function: str }));
    typ
}
