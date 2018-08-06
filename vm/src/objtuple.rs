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
            PyObjectKind::Tuple {
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
