use super::super::pyobject::{
    IdProtocol, PyObject, PyObjectPayload, PyObjectRef, PyResult, TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objbool;
use super::objint;
use num_bigint::BigInt;
use num_traits::{One, Signed, ToPrimitive, Zero};
use std::cell::{Ref, RefMut};
use std::marker::Sized;
use std::ops::{Deref, DerefMut, Range};

pub trait PySliceableSequence {
    fn do_slice(&self, range: Range<usize>) -> Self;
    fn do_slice_reverse(&self, range: Range<usize>) -> Self;
    fn do_stepped_slice(&self, range: Range<usize>, step: usize) -> Self;
    fn do_stepped_slice_reverse(&self, range: Range<usize>, step: usize) -> Self;
    fn empty() -> Self;
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool;
    fn get_pos(&self, p: i32) -> Option<usize> {
        if p < 0 {
            if -p as usize > self.len() {
                None
            } else {
                Some(self.len() - ((-p) as usize))
            }
        } else if p as usize >= self.len() {
            None
        } else {
            Some(p as usize)
        }
    }

    fn get_slice_pos(&self, slice_pos: &BigInt) -> usize {
        if let Some(pos) = slice_pos.to_i32() {
            if let Some(index) = self.get_pos(pos) {
                // within bounds
                return index;
            }
        }

        if slice_pos.is_negative() {
            0
        } else {
            self.len()
        }
    }

    fn get_slice_range(&self, start: &Option<BigInt>, stop: &Option<BigInt>) -> Range<usize> {
        let start = start.as_ref().map(|x| self.get_slice_pos(x)).unwrap_or(0);
        let stop = stop
            .as_ref()
            .map(|x| self.get_slice_pos(x))
            .unwrap_or_else(|| self.len());

        start..stop
    }

    fn get_slice_items(
        &self,
        vm: &mut VirtualMachine,
        slice: &PyObjectRef,
    ) -> Result<Self, PyObjectRef>
    where
        Self: Sized,
    {
        // TODO: we could potentially avoid this copy and use slice
        match &(slice.borrow()).payload {
            PyObjectPayload::Slice { start, stop, step } => {
                let step = step.clone().unwrap_or_else(BigInt::one);
                if step.is_zero() {
                    Err(vm.new_value_error("slice step cannot be zero".to_string()))
                } else if step.is_positive() {
                    let range = self.get_slice_range(start, stop);
                    if range.start < range.end {
                        match step.to_i32() {
                            Some(1) => Ok(self.do_slice(range)),
                            Some(num) => Ok(self.do_stepped_slice(range, num as usize)),
                            None => Ok(self.do_slice(range.start..range.start + 1)),
                        }
                    } else {
                        Ok(Self::empty())
                    }
                } else {
                    // calculate the range for the reverse slice, first the bounds needs to be made
                    // exclusive around stop, the lower number
                    let start = start.as_ref().map(|x| x + 1);
                    let stop = stop.as_ref().map(|x| x + 1);
                    let range = self.get_slice_range(&stop, &start);
                    if range.start < range.end {
                        match (-step).to_i32() {
                            Some(1) => Ok(self.do_slice_reverse(range)),
                            Some(num) => Ok(self.do_stepped_slice_reverse(range, num as usize)),
                            None => Ok(self.do_slice(range.end - 1..range.end)),
                        }
                    } else {
                        Ok(Self::empty())
                    }
                }
            }
            payload => panic!("get_slice_items called with non-slice: {:?}", payload),
        }
    }
}

impl<T: Clone> PySliceableSequence for Vec<T> {
    fn do_slice(&self, range: Range<usize>) -> Self {
        self[range].to_vec()
    }

    fn do_slice_reverse(&self, range: Range<usize>) -> Self {
        let mut slice = self[range].to_vec();
        slice.reverse();
        slice
    }

    fn do_stepped_slice(&self, range: Range<usize>, step: usize) -> Self {
        self[range].iter().step_by(step).cloned().collect()
    }

    fn do_stepped_slice_reverse(&self, range: Range<usize>, step: usize) -> Self {
        self[range].iter().rev().step_by(step).cloned().collect()
    }

    fn empty() -> Self {
        Vec::new()
    }

    fn len(&self) -> usize {
        self.len()
    }

    fn is_empty(&self) -> bool {
        self.is_empty()
    }
}

pub fn get_item(
    vm: &mut VirtualMachine,
    sequence: &PyObjectRef,
    elements: &[PyObjectRef],
    subscript: PyObjectRef,
) -> PyResult {
    match &(subscript.borrow()).payload {
        PyObjectPayload::Integer { value } => match value.to_i32() {
            Some(value) => {
                if let Some(pos_index) = elements.to_vec().get_pos(value) {
                    let obj = elements[pos_index].clone();
                    Ok(obj)
                } else {
                    Err(vm.new_index_error("Index out of bounds!".to_string()))
                }
            }
            None => {
                Err(vm.new_index_error("cannot fit 'int' into an index-sized integer".to_string()))
            }
        },

        PyObjectPayload::Slice { .. } => Ok(PyObject::new(
            match &(sequence.borrow()).payload {
                PyObjectPayload::Sequence { .. } => PyObjectPayload::Sequence {
                    elements: elements.to_vec().get_slice_items(vm, &subscript)?,
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
    zelf: &[PyObjectRef],
    other: &[PyObjectRef],
) -> Result<bool, PyObjectRef> {
    if zelf.len() == other.len() {
        for (a, b) in Iterator::zip(zelf.iter(), other.iter()) {
            if !a.is(&b) {
                let eq = vm._eq(a.clone(), b.clone())?;
                let value = objbool::boolval(vm, eq)?;
                if !value {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    } else {
        Ok(false)
    }
}

pub fn seq_lt(
    vm: &mut VirtualMachine,
    zelf: &[PyObjectRef],
    other: &[PyObjectRef],
) -> Result<bool, PyObjectRef> {
    if zelf.len() == other.len() {
        for (a, b) in Iterator::zip(zelf.iter(), other.iter()) {
            let lt = vm._lt(a.clone(), b.clone())?;
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
            let lt = vm._lt(a.clone(), b.clone())?;
            let eq = vm._eq(a.clone(), b.clone())?;
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
    zelf: &[PyObjectRef],
    other: &[PyObjectRef],
) -> Result<bool, PyObjectRef> {
    if zelf.len() == other.len() {
        for (a, b) in Iterator::zip(zelf.iter(), other.iter()) {
            let gt = vm._gt(a.clone(), b.clone())?;
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
            let gt = vm._gt(a.clone(), b.clone())?;
            let eq = vm._eq(a.clone(), b.clone())?;
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
    zelf: &[PyObjectRef],
    other: &[PyObjectRef],
) -> Result<bool, PyObjectRef> {
    Ok(seq_gt(vm, zelf, other)? || seq_equal(vm, zelf, other)?)
}

pub fn seq_le(
    vm: &mut VirtualMachine,
    zelf: &[PyObjectRef],
    other: &[PyObjectRef],
) -> Result<bool, PyObjectRef> {
    Ok(seq_lt(vm, zelf, other)? || seq_equal(vm, zelf, other)?)
}

pub fn seq_mul(elements: &[PyObjectRef], product: &PyObjectRef) -> Vec<PyObjectRef> {
    let counter = objint::get_value(&product).to_isize().unwrap();

    let current_len = elements.len();
    let new_len = counter.max(0) as usize * current_len;
    let mut new_elements = Vec::with_capacity(new_len);

    for _ in 0..counter {
        new_elements.extend(elements.to_owned());
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
