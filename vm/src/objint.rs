use super::pyobject::{PyObject, PyObjectKind, PyObjectRef};

fn str(args: Vec<PyObjectRef>) -> Result<PyObjectRef, PyObjectRef> {
    Ok(PyObject::new(PyObjectKind::String {
        value: "todo".to_string(),
    }))
}

fn add() {}

/*
fn set_attr(a: &mut PyObjectRef, name: String, b: PyObjectRef) {
    a.borrow().dict.insert(name, b);
}
*/

pub fn create_type() -> PyObjectRef {
    let mut typ = PyObject::new(PyObjectKind::Type);
    typ.borrow_mut().dict.insert(
        "__str__".to_string(),
        PyObject::new(PyObjectKind::RustFunction { function: str }),
    );
    typ
}
