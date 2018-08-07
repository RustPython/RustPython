use super::pyobject::{PyObject, PyObjectKind, PyObjectRef, PyResult};
use super::vm::VirtualMachine;

pub fn get_pos(l: &Vec<PyObjectRef>, p: i32) -> usize {
    if p < 0 {
        l.len() - ((-p) as usize)
    } else {
        p as usize
    }
}

fn get_slice_items(l: &Vec<PyObjectRef>, slice: &PyObjectRef) -> Vec<PyObjectRef> {
    // TODO: we could potentially avoid this copy and use slice
    match &(slice.borrow()).kind {
        PyObjectKind::Slice { start, stop, step } => {
            let start = match start {
                &Some(start) => get_pos(l, start),
                &None => 0,
            };
            let stop = match stop {
                &Some(stop) => get_pos(l, stop),
                &None => l.len() as usize,
            };
            match step {
                &None | &Some(1) => l[start..stop].to_vec(),
                &Some(num) => {
                    if num < 0 {
                        unimplemented!("negative step indexing not yet supported")
                    };
                    l[start..stop]
                        .iter()
                        .step_by(num as usize)
                        .cloned()
                        .collect()
                }
            }
        }
        kind => panic!("get_slice_items called with non-slice: {:?}", kind),
    }
}

pub fn get_item(
    vm: &mut VirtualMachine,
    sequence: &PyObjectRef,
    elements: &Vec<PyObjectRef>,
    subscript: PyObjectRef,
) -> PyResult {
    match &(subscript.borrow()).kind {
        PyObjectKind::Integer { value } => {
            let pos_index = get_pos(elements, *value);
            if pos_index < elements.len() {
                let obj = elements[pos_index].clone();
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
            match &(sequence.borrow()).kind {
                PyObjectKind::Tuple { elements: _ } => PyObjectKind::Tuple {
                    elements: get_slice_items(elements, &subscript),
                },
                PyObjectKind::List { elements: _ } => PyObjectKind::List {
                    elements: get_slice_items(elements, &subscript),
                },
                ref kind => panic!("sequence get_item called for non-sequence: {:?}", kind),
            },
            vm.get_type(),
        )),
        _ => Err(vm.new_exception(format!(
            "TypeError: indexing type {:?} with index {:?} is not supported (yet?)",
            sequence, subscript
        ))),
    }
}
