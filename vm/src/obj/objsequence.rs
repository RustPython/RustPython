use crate::function::OptionalArg;
use crate::obj::objnone::PyNone;
use std::cell::RefCell;
use std::marker::Sized;
use std::ops::{Deref, DerefMut, Range};

use crate::pyobject::{IdProtocol, PyObject, PyObjectRef, PyResult, TryFromObject, TypeProtocol};

use crate::vm::VirtualMachine;
use num_bigint::{BigInt, ToBigInt};
use num_traits::{One, Signed, ToPrimitive, Zero};

use super::objbool;
use super::objint::PyInt;
use super::objlist::PyList;
use super::objslice::{PySlice, PySliceRef};
use super::objtuple::PyTuple;

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

    fn get_slice_items(&self, vm: &VirtualMachine, slice: &PyObjectRef) -> Result<Self, PyObjectRef>
    where
        Self: Sized,
    {
        match slice.clone().downcast::<PySlice>() {
            Ok(slice) => {
                let start = slice.start_index(vm)?;
                let stop = slice.stop_index(vm)?;
                let step = slice.step_index(vm)?.unwrap_or_else(BigInt::one);
                if step.is_zero() {
                    Err(vm.new_value_error("slice step cannot be zero".to_string()))
                } else if step.is_positive() {
                    let range = self.get_slice_range(&start, &stop);
                    if range.start < range.end {
                        #[allow(clippy::range_plus_one)]
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
                    let start = start.as_ref().map(|x| {
                        if *x == (-1).to_bigint().unwrap() {
                            self.len() + BigInt::one() //.to_bigint().unwrap()
                        } else {
                            x + 1
                        }
                    });
                    let stop = stop.as_ref().map(|x| {
                        if *x == (-1).to_bigint().unwrap() {
                            self.len().to_bigint().unwrap()
                        } else {
                            x + 1
                        }
                    });
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

pub enum SequenceIndex {
    Int(i32),
    Slice(PySliceRef),
}

impl TryFromObject for SequenceIndex {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        match_class!(obj,
            i @ PyInt => Ok(SequenceIndex::Int(i32::try_from_object(vm, i.into_object())?)),
            s @ PySlice => Ok(SequenceIndex::Slice(s)),
            obj => Err(vm.new_type_error(format!(
                "sequence indices be integers or slices, not {}",
                obj.class(),
            )))
        )
    }
}

pub fn get_item(
    vm: &VirtualMachine,
    sequence: &PyObjectRef,
    elements: &[PyObjectRef],
    subscript: PyObjectRef,
) -> PyResult {
    if let Some(i) = subscript.payload::<PyInt>() {
        return match i.as_bigint().to_i32() {
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
        };
    }

    if subscript.payload::<PySlice>().is_some() {
        if sequence.payload::<PyList>().is_some() {
            Ok(PyObject::new(
                PyList::from(elements.to_vec().get_slice_items(vm, &subscript)?),
                sequence.class(),
                None,
            ))
        } else if sequence.payload::<PyTuple>().is_some() {
            Ok(PyObject::new(
                PyTuple::from(elements.to_vec().get_slice_items(vm, &subscript)?),
                sequence.class(),
                None,
            ))
        } else {
            panic!("sequence get_item called for non-sequence")
        }
    } else {
        Err(vm.new_type_error(format!(
            "TypeError: indexing type {:?} with index {:?} is not supported (yet?)",
            sequence, subscript
        )))
    }
}

pub fn seq_equal(
    vm: &VirtualMachine,
    zelf: &[PyObjectRef],
    other: &[PyObjectRef],
) -> Result<bool, PyObjectRef> {
    if zelf.len() == other.len() {
        for (a, b) in Iterator::zip(zelf.iter(), other.iter()) {
            if !a.is(b) {
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
    vm: &VirtualMachine,
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
    vm: &VirtualMachine,
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
    vm: &VirtualMachine,
    zelf: &[PyObjectRef],
    other: &[PyObjectRef],
) -> Result<bool, PyObjectRef> {
    Ok(seq_gt(vm, zelf, other)? || seq_equal(vm, zelf, other)?)
}

pub fn seq_le(
    vm: &VirtualMachine,
    zelf: &[PyObjectRef],
    other: &[PyObjectRef],
) -> Result<bool, PyObjectRef> {
    Ok(seq_lt(vm, zelf, other)? || seq_equal(vm, zelf, other)?)
}

pub fn seq_mul(elements: &[PyObjectRef], counter: isize) -> Vec<PyObjectRef> {
    let current_len = elements.len();
    let new_len = counter.max(0) as usize * current_len;
    let mut new_elements = Vec::with_capacity(new_len);

    for _ in 0..counter {
        new_elements.extend(elements.to_owned());
    }

    new_elements
}

pub fn get_elements_cell<'a>(obj: &'a PyObjectRef) -> &'a RefCell<Vec<PyObjectRef>> {
    if let Some(list) = obj.payload::<PyList>() {
        return &list.elements;
    }
    if let Some(tuple) = obj.payload::<PyTuple>() {
        return &tuple.elements;
    }
    panic!("Cannot extract elements from non-sequence");
}

pub fn get_elements<'a>(obj: &'a PyObjectRef) -> impl Deref<Target = Vec<PyObjectRef>> + 'a {
    if let Some(list) = obj.payload::<PyList>() {
        return list.elements.borrow();
    }
    if let Some(tuple) = obj.payload::<PyTuple>() {
        return tuple.elements.borrow();
    }
    panic!("Cannot extract elements from non-sequence");
}

pub fn get_mut_elements<'a>(obj: &'a PyObjectRef) -> impl DerefMut<Target = Vec<PyObjectRef>> + 'a {
    if let Some(list) = obj.payload::<PyList>() {
        return list.elements.borrow_mut();
    }
    if let Some(tuple) = obj.payload::<PyTuple>() {
        return tuple.elements.borrow_mut();
    }
    panic!("Cannot extract elements from non-sequence");
}

//Check if given arg could be used with PySciceableSequance.get_slice_range()
pub fn is_valid_slice_arg(
    arg: OptionalArg<PyObjectRef>,
    vm: &VirtualMachine,
) -> Result<Option<BigInt>, PyObjectRef> {
    if let OptionalArg::Present(value) = arg {
        match_class!(value,
        i @ PyInt => Ok(Some(i.as_bigint().clone())),
        _obj @ PyNone => Ok(None),
        _=> {return Err(vm.new_type_error("slice indices must be integers or None or have an __index__ method".to_string()));}
        // TODO: check for an __index__ method
        )
    } else {
        Ok(None)
    }
}
