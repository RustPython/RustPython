use super::objsequence::PySliceableSequence;
use super::pyobject::{PyFuncArgs, PyObject, PyObjectKind, PyObjectRef, PyResult};
use super::vm::VirtualMachine;
use std::collections::HashMap;

// set_item:
pub fn set_item(
    vm: &mut VirtualMachine,
    l: &mut Vec<PyObjectRef>,
    idx: PyObjectRef,
    obj: PyObjectRef,
) -> PyResult {
    match &(idx.borrow()).kind {
        PyObjectKind::Integer { value } => {
            let pos_index = l.get_pos(*value);
            l[pos_index] = obj;
            Ok(vm.get_none())
        }
        _ => panic!(
            "TypeError: indexing type {:?} with index {:?} is not supported (yet?)",
            l, idx
        ),
    }
}

fn append(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    // TODO: Implement objlist::append
    trace!("list.append called with: {:?}", args);
    if let PyObjectKind::List { ref elements } = args.args[0].borrow().kind {
        warn!("implement append here: {:?}", elements);
        // elements.push(args.args[1].clone());
        Ok(vm.get_none())
    } else {
        unimplemented!("Raise error when list.append is called with no list")
        // Err()
    }
    // Ok(vm.new_bound_method(args.args[0].clone(), args.args[1].clone()))
}

pub fn create_type(type_type: PyObjectRef) -> PyObjectRef {
    let mut dict = HashMap::new();
    dict.insert(
        "append".to_string(),
        PyObject::new(
            PyObjectKind::RustFunction { function: append },
            type_type.clone(),
        ),
    );
    let typ = PyObject::new(
        PyObjectKind::Class {
            name: "list".to_string(),
            dict: PyObject::new(PyObjectKind::Dict { elements: dict }, type_type.clone()),
        },
        type_type.clone(),
    );
    typ
}
