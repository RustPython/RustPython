use super::pyobject::{PyObject, PyObjectKind, PyObjectRef, PyResult};
use super::vm::VirtualMachine;

pub fn get_pos(sequence_length: usize, p: i32) -> usize {
    if p < 0 {
        sequence_length - ((-p) as usize)
    } else if p as usize > sequence_length {
        // This is for the slicing case where the end element is greater than the length of the
        // sequence
        sequence_length
    } else {
        p as usize
    }
}

pub trait PySliceableSequence {
    fn do_slice(&self, start: usize, stop: usize) -> Self;
    fn do_stepped_slice(&self, start: usize, stop: usize, step: usize) -> Self;
    fn len(&self) -> usize;
}

impl PySliceableSequence for Vec<PyObjectRef> {
    fn do_slice(&self, start: usize, stop: usize) -> Self {
        self[start..stop].to_vec()
    }
    fn do_stepped_slice(&self, start: usize, stop: usize, step: usize) -> Self {
        self[start..stop].iter().step_by(step).cloned().collect()
    }
    fn len(&self) -> usize {
        self.len()
    }
}

pub fn get_slice_items<S>(l: &S, slice: &PyObjectRef) -> S
where
    S: PySliceableSequence,
{
    // TODO: we could potentially avoid this copy and use slice
    match &(slice.borrow()).kind {
        PyObjectKind::Slice { start, stop, step } => {
            let start = match start {
                &Some(start) => get_pos(l.len(), start),
                &None => 0,
            };
            let stop = match stop {
                &Some(stop) => get_pos(l.len(), stop),
                &None => l.len() as usize,
            };
            match step {
                &None | &Some(1) => l.do_slice(start, stop),
                &Some(num) => {
                    if num < 0 {
                        unimplemented!("negative step indexing not yet supported")
                    };
                    l.do_stepped_slice(start, stop, num as usize)
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
            let pos_index = get_pos(elements.len(), *value);
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
