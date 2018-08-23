use super::pyobject::{PyFuncArgs, PyObject, PyObjectKind, PyObjectRef, PyResult};
use super::vm::VirtualMachine;
use std::collections::HashMap;

fn init(vm: &mut VirtualMachine, _args: PyFuncArgs) -> PyResult {
    Ok(vm.get_none())
}

pub fn create_base_exception_type(type_type: PyObjectRef, object_type: PyObjectRef) -> PyObjectRef {
    let mut dict = HashMap::new();
    dict.insert(
        "__init__".to_string(),
        PyObject::new(
            PyObjectKind::RustFunction { function: init },
            type_type.clone(),
        ),
    );
    let typ = PyObject::new(
        PyObjectKind::Class {
            name: "BaseException".to_string(),
            dict: PyObject::new(PyObjectKind::Dict { elements: dict }, type_type.clone()),
            mro: vec![object_type],
        },
        type_type.clone(),
    );
    typ
}

/*
 * TODO: create a whole exception hierarchy somehow?
pub fn create_exception_zoo(context: &PyContext) {
    let base_exception_type = PyObjectKind::Class {
        name: String::from("Exception"),
        dict: context.new_dict(),
        mro: vec![object_type],
    }.into_ref();
}
*/
