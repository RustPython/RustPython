use super::pyobject::{PyObject, PyObjectKind, PyObjectRef, PyResult};
use super::vm::VirtualMachine;

pub fn get_item(vm: &mut VirtualMachine, l: &Vec<PyObjectRef>, b: PyObjectRef) -> PyResult {
    match &(b.borrow()).kind {
        PyObjectKind::Integer { value } => {
            let pos_index = super::objsequence::get_pos(l, *value);
            if pos_index < l.len() {
                let obj = l[pos_index].clone();
                Ok(obj)
            } else {
                Err(vm.new_exception("Index out of bounds!".to_string()))
            }
        }
        PyObjectKind::Slice {
            start: _,
            stop: _,
            step: _,
        } => Ok(PyObject::new(
            PyObjectKind::List {
                elements: super::objsequence::get_slice_items(l, &b),
            },
            vm.get_type(),
        )),
        _ => Err(vm.new_exception(format!(
            "TypeError: indexing type {:?} with index {:?} is not supported (yet?)",
            l, b
        ))),
    }
}

// set_item:
pub fn set_item(
    vm: &mut VirtualMachine,
    l: &mut Vec<PyObjectRef>,
    idx: PyObjectRef,
    obj: PyObjectRef,
) -> PyResult {
    match &(idx.borrow()).kind {
        PyObjectKind::Integer { value } => {
            let pos_index = super::objsequence::get_pos(l, *value);
            l[pos_index] = obj;
            Ok(vm.get_none())
        }
        _ => panic!(
            "TypeError: indexing type {:?} with index {:?} is not supported (yet?)",
            l, idx
        ),
    }
}

pub fn append(vm: &mut VirtualMachine, l: PyObjectRef, other: PyObjectRef) -> PyResult {
    Ok(vm.get_none())
}

/* TODO:
pub fn make_type() -> PyObjectRef {

    // dict.insert("__set_item__".to_string(), set_item);
    dict.insert("append".to_string(), append)
}
*/
