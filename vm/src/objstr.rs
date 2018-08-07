use super::objsequence;
use super::pyobject::{PyObjectKind, PyObjectRef, PyResult};
use super::vm::VirtualMachine;

impl objsequence::PySliceableSequence for String {
    fn do_slice(&self, start: usize, stop: usize) -> Self {
        self[start..stop].to_string()
    }
    fn do_stepped_slice(&self, start: usize, stop: usize, step: usize) -> Self {
        self[start..stop].chars().step_by(step).collect()
    }
    fn len(&self) -> usize {
        self.len()
    }
}

pub fn subscript(vm: &mut VirtualMachine, value: &String, b: PyObjectRef) -> PyResult {
    // let value = a
    match &(*b.borrow()).kind {
        &PyObjectKind::Integer { value: ref pos } => {
            let idx = objsequence::get_pos(value.len(), *pos);
            Ok(vm.new_str(value[idx..idx + 1].to_string()))
        }
        &PyObjectKind::Slice {
            start: _,
            stop: _,
            step: _,
        } => Ok(vm.new_str(objsequence::get_slice_items(value, &b))),
        _ => panic!(
            "TypeError: indexing type {:?} with index {:?} is not supported (yet?)",
            value, b
        ),
    }
}
