use super::objsequence;
use super::pyobject::{PyObjectKind, PyObjectRef, PyResult};
use super::vm::VirtualMachine;

fn get_slice_items(value: &String, slice: &PyObjectRef) -> String {
    match &(slice.borrow()).kind {
        PyObjectKind::Slice { start, stop, step } => {
            let start2: usize = match start {
                // &Some(_) => panic!("Bad start index for string slicing {:?}", start),
                &Some(start) => objsequence::get_pos(value.len(), start),
                &None => 0,
            };
            let stop2: usize = match stop {
                &Some(stop) => objsequence::get_pos(value.len(), stop),
                // &Some(_) => panic!("Bad stop index for string slicing"),
                &None => value.len() as usize,
            };
            match step {
                &None | &Some(1) => value[start2..stop2].to_string(),
                &Some(num) => {
                    if num < 0 {
                        unimplemented!("negative step indexing not yet supported")
                    };
                    value[start2..stop2].chars().step_by(num as usize).collect()
                }
            }
        }
        kind => panic!("get_slice_items called with non-slice: {:?}", kind),
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
        } => Ok(vm.new_str(get_slice_items(value, &b))),
        _ => panic!(
            "TypeError: indexing type {:?} with index {:?} is not supported (yet?)",
            value, b
        ),
    }
}
