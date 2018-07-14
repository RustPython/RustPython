use super::pyobject::{PyObject, PyObjectKind, PyObjectRef, Executor};

fn str(rt: &mut Executor, args: Vec<PyObjectRef>) -> Result<PyObjectRef, PyObjectRef> {
    Ok(rt.new_str("todo".to_string()))
}

fn add() {}

/*
fn set_attr(a: &mut PyObjectRef, name: String, b: PyObjectRef) {
    a.borrow().dict.insert(name, b);
}
*/

pub fn create_type(type_type: PyObjectRef) -> PyObjectRef {
    let typ = PyObject::new(PyObjectKind::Class { name: "int".to_string() }, type_type.clone());
    typ.borrow_mut().dict.insert(
        "__str__".to_string(),
        PyObject::new(PyObjectKind::RustFunction { function: str }, type_type.clone()),
    );
    typ
}
