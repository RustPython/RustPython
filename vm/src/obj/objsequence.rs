use super::super::pyobject::{PyObject, PyObjectKind, PyObjectRef, PyResult, TypeProtocol};
use super::super::vm::VirtualMachine;
use super::objbool;
use std::marker::Sized;

pub trait PySliceableSequence {
    fn do_slice(&self, start: usize, stop: usize) -> Self;
    fn do_stepped_slice(&self, start: usize, stop: usize, step: usize) -> Self;
    fn len(&self) -> usize;
    fn get_pos(&self, p: i32) -> usize {
        if p < 0 {
            self.len() - ((-p) as usize)
        } else if p as usize > self.len() {
            // This is for the slicing case where the end element is greater than the length of the
            // sequence
            self.len()
        } else {
            p as usize
        }
    }
    fn get_slice_items(&self, slice: &PyObjectRef) -> Self
    where
        Self: Sized,
    {
        // TODO: we could potentially avoid this copy and use slice
        match &(slice.borrow()).kind {
            PyObjectKind::Slice { start, stop, step } => {
                let start = match start {
                    &Some(start) => self.get_pos(start),
                    &None => 0,
                };
                let stop = match stop {
                    &Some(stop) => self.get_pos(stop),
                    &None => self.len() as usize,
                };
                match step {
                    &None | &Some(1) => self.do_slice(start, stop),
                    &Some(num) => {
                        if num < 0 {
                            unimplemented!("negative step indexing not yet supported")
                        };
                        self.do_stepped_slice(start, stop, num as usize)
                    }
                }
            }
            kind => panic!("get_slice_items called with non-slice: {:?}", kind),
        }
    }
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

pub fn get_item(
    vm: &mut VirtualMachine,
    sequence: &PyObjectRef,
    elements: &[PyObjectRef],
    subscript: PyObjectRef,
) -> PyResult {
    match &(subscript.borrow()).kind {
        PyObjectKind::Integer { value } => {
            let pos_index = elements.to_vec().get_pos(*value);
            if pos_index < elements.len() {
                let obj = elements[pos_index].clone();
                Ok(obj)
            } else {
                let value_error = vm.context().exceptions.value_error.clone();
                Err(vm.new_exception(value_error, "Index out of bounds!".to_string()))
            }
        }
        PyObjectKind::Slice {
            start: _,
            stop: _,
            step: _,
        } => Ok(PyObject::new(
            match &(sequence.borrow()).kind {
                PyObjectKind::Tuple { elements: _ } => PyObjectKind::Tuple {
                    elements: elements.to_vec().get_slice_items(&subscript),
                },
                PyObjectKind::List { elements: _ } => PyObjectKind::List {
                    elements: elements.to_vec().get_slice_items(&subscript),
                },
                ref kind => panic!("sequence get_item called for non-sequence: {:?}", kind),
            },
            sequence.typ(),
        )),
        _ => Err(vm.new_type_error(format!(
            "TypeError: indexing type {:?} with index {:?} is not supported (yet?)",
            sequence, subscript
        ))),
    }
}

pub fn seq_equal(
    vm: &mut VirtualMachine,
    zelf: Vec<PyObjectRef>,
    other: Vec<PyObjectRef>,
) -> Result<bool, PyObjectRef> {
    if zelf.len() == other.len() {
        for (a, b) in Iterator::zip(zelf.iter(), other.iter()) {
            let eq = vm.call_method(&a.clone(), "__eq__", vec![b.clone()])?;
            let value = objbool::boolval(vm, eq)?;
            if !value {
                return Ok(false);
            }
        }
        Ok(true)
    } else {
        Ok(false)
    }
}
