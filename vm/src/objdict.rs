use super::pyobject::{PyObject, PyObjectKind, PyObjectRef, PyResult};
use super::vm::VirtualMachine;
use std::collections::HashMap;

pub fn _set_item(
    vm: &mut VirtualMachine,
    _d: PyObjectRef,
    _idx: PyObjectRef,
    _obj: PyObjectRef,
) -> PyResult {
    // TODO: Implement objdict::set_item
    Ok(vm.get_none())
}

pub fn new(dict_type: PyObjectRef) -> PyObjectRef {
    PyObject::new(
        PyObjectKind::Dict {
            elements: HashMap::new(),
        },
        dict_type.clone(),
    )
}

pub fn create_type(type_type: PyObjectRef, object_type: PyObjectRef, dict_type: PyObjectRef) {
    (*dict_type.borrow_mut()).kind = PyObjectKind::Class {
        name: String::from("type"),
        dict: new(dict_type.clone()),
        mro: vec![object_type],
    };
    (*dict_type.borrow_mut()).typ = Some(type_type.clone());
}

/* TODO:
pub fn make_type() -> PyObjectRef {

    // dict.insert("__set_item__".to_string(), _set_item);
}
*/
