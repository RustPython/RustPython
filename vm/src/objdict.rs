use super::objtype;
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

pub fn create_type(type_type: PyObjectRef, object: PyObjectRef) -> PyObjectRef {
    let dict = PyObject {
        kind: PyObjectKind::Dict {
            elements: HashMap::new(),
        },
        typ: None,
    }.into_ref();
    let dict_type = objtype::new(
        type_type.clone(),
        "dict",
        vec![object.clone()],
        dict.clone(),
    ).unwrap();
    (*dict.borrow_mut()).typ = Some(dict_type.clone());
    dict_type
}

/* TODO:
pub fn make_type() -> PyObjectRef {

    // dict.insert("__set_item__".to_string(), _set_item);
}
*/
