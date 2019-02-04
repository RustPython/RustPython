use super::super::pyobject::{PyObject, PyObjectPayload, PyObjectRef, PyResult, TypeProtocol};
use super::super::vm::VirtualMachine;
use super::objbool;
use super::objint;
use num_traits::ToPrimitive;
use std::cell::{Ref, RefMut};
use std::marker::Sized;
use std::ops::{Deref, DerefMut};

pub trait PySliceableSequence {
    fn do_slice(&self, start: usize, stop: usize) -> Self;
    fn do_stepped_slice(&self, start: usize, stop: usize, step: usize) -> Self;
    fn len(&self) -> usize;
    fn get_pos(&self, p: i32) -> usize {
        if p < 0 {
            if -p as usize > self.len() {
                // return something that is out of bounds so `get_item` raises an IndexError
                self.len() + 1
            } else {
                self.len() - ((-p) as usize)
            }
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
        match &(slice.borrow()).payload {
            PyObjectPayload::Slice { start, stop, step } => {
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
            payload => panic!("get_slice_items called with non-slice: {:?}", payload),
        }
    }
}

impl<T: Clone> PySliceableSequence for Vec<T> {
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
    match &(subscript.borrow()).payload {
        PyObjectPayload::Integer { value } => {
            let value = value.to_i32().unwrap();
            let pos_index = elements.to_vec().get_pos(value);
            if pos_index < elements.len() {
                let obj = elements[pos_index].clone();
                Ok(obj)
            } else {
                let index_error = vm.context().exceptions.index_error.clone();
                Err(vm.new_exception(index_error, "Index out of bounds!".to_string()))
            }
        }
        PyObjectPayload::Slice {
            start: _,
            stop: _,
            step: _,
        } => Ok(PyObject::new(
            match &(sequence.borrow()).payload {
                PyObjectPayload::Sequence { elements: _ } => PyObjectPayload::Sequence {
                    elements: elements.to_vec().get_slice_items(&subscript),
                },
                ref payload => panic!("sequence get_item called for non-sequence: {:?}", payload),
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
    zelf: &Vec<PyObjectRef>,
    other: &Vec<PyObjectRef>,
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

pub fn seq_lt(
    vm: &mut VirtualMachine,
    zelf: &Vec<PyObjectRef>,
    other: &Vec<PyObjectRef>,
) -> Result<bool, PyObjectRef> {
    if zelf.len() == other.len() {
        for (a, b) in Iterator::zip(zelf.iter(), other.iter()) {
            let lt = vm.call_method(&a.clone(), "__lt__", vec![b.clone()])?;
            let value = objbool::boolval(vm, lt)?;
            if !value {
                return Ok(false);
            }
        }
        Ok(true)
    } else {
        // This case is more complicated because it can still return true if
        // `zelf` is the head of `other` e.g. [1,2,3] < [1,2,3,4] should return true
        let mut head = true; // true if `zelf` is the head of `other`

        for (a, b) in Iterator::zip(zelf.iter(), other.iter()) {
            let lt = vm.call_method(&a.clone(), "__lt__", vec![b.clone()])?;
            let eq = vm.call_method(&a.clone(), "__eq__", vec![b.clone()])?;
            let lt_value = objbool::boolval(vm, lt)?;
            let eq_value = objbool::boolval(vm, eq)?;

            if !lt_value && !eq_value {
                return Ok(false);
            } else if !eq_value {
                head = false;
            }
        }

        if head {
            Ok(zelf.len() < other.len())
        } else {
            Ok(true)
        }
    }
}

pub fn seq_gt(
    vm: &mut VirtualMachine,
    zelf: &Vec<PyObjectRef>,
    other: &Vec<PyObjectRef>,
) -> Result<bool, PyObjectRef> {
    if zelf.len() == other.len() {
        for (a, b) in Iterator::zip(zelf.iter(), other.iter()) {
            let gt = vm.call_method(&a.clone(), "__gt__", vec![b.clone()])?;
            let value = objbool::boolval(vm, gt)?;
            if !value {
                return Ok(false);
            }
        }
        Ok(true)
    } else {
        let mut head = true; // true if `other` is the head of `zelf`
        for (a, b) in Iterator::zip(zelf.iter(), other.iter()) {
            // This case is more complicated because it can still return true if
            // `other` is the head of `zelf` e.g. [1,2,3,4] > [1,2,3] should return true
            let gt = vm.call_method(&a.clone(), "__gt__", vec![b.clone()])?;
            let eq = vm.call_method(&a.clone(), "__eq__", vec![b.clone()])?;
            let gt_value = objbool::boolval(vm, gt)?;
            let eq_value = objbool::boolval(vm, eq)?;

            if !gt_value && !eq_value {
                return Ok(false);
            } else if !eq_value {
                head = false;
            }
        }

        if head {
            Ok(zelf.len() > other.len())
        } else {
            Ok(true)
        }
    }
}

pub fn seq_ge(
    vm: &mut VirtualMachine,
    zelf: &Vec<PyObjectRef>,
    other: &Vec<PyObjectRef>,
) -> Result<bool, PyObjectRef> {
    Ok(seq_gt(vm, zelf, other)? || seq_equal(vm, zelf, other)?)
}

pub fn seq_le(
    vm: &mut VirtualMachine,
    zelf: &Vec<PyObjectRef>,
    other: &Vec<PyObjectRef>,
) -> Result<bool, PyObjectRef> {
    Ok(seq_lt(vm, zelf, other)? || seq_equal(vm, zelf, other)?)
}

pub fn seq_mul(elements: &Vec<PyObjectRef>, product: &PyObjectRef) -> Vec<PyObjectRef> {
    let counter = objint::get_value(&product).to_isize().unwrap();

    let current_len = elements.len();
    let new_len = counter.max(0) as usize * current_len;
    let mut new_elements = Vec::with_capacity(new_len);

    for _ in 0..counter {
        new_elements.extend(elements.clone());
    }

    new_elements
}

pub fn get_elements<'a>(obj: &'a PyObjectRef) -> impl Deref<Target = Vec<PyObjectRef>> + 'a {
    Ref::map(obj.borrow(), |x| {
        if let PyObjectPayload::Sequence { ref elements } = x.payload {
            elements
        } else {
            panic!("Cannot extract elements from non-sequence");
        }
    })
}

pub fn get_mut_elements<'a>(obj: &'a PyObjectRef) -> impl DerefMut<Target = Vec<PyObjectRef>> + 'a {
    RefMut::map(obj.borrow_mut(), |x| {
        if let PyObjectPayload::Sequence { ref mut elements } = x.payload {
            elements
        } else {
            panic!("Cannot extract list elements from non-sequence");
            // TODO: raise proper error?
            // Err(vm.new_type_error("list.append is called with no list".to_string()))
        }
    })
}
