use std::cell::RefCell;
use std::marker::Sized;
use std::ops::{Deref, DerefMut, Range};

use num_bigint::{BigInt, ToBigInt};
use num_traits::{One, Signed, ToPrimitive, Zero};

use super::objint::{PyInt, PyIntRef};
use super::objlist::PyList;
use super::objnone::PyNone;
use super::objslice::{PySlice, PySliceRef};
use super::objtuple::PyTuple;
use crate::function::OptionalArg;
use crate::pyobject::{IdProtocol, PyObject, PyObjectRef, PyResult, TryFromObject, TypeProtocol};
use crate::vm::VirtualMachine;

pub trait PySliceableSequence {
    type Sliced;

    fn do_slice(&self, range: Range<usize>) -> Self::Sliced;
    fn do_slice_reverse(&self, range: Range<usize>) -> Self::Sliced;
    fn do_stepped_slice(&self, range: Range<usize>, step: usize) -> Self::Sliced;
    fn do_stepped_slice_reverse(&self, range: Range<usize>, step: usize) -> Self::Sliced;
    fn empty() -> Self::Sliced;

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

    fn get_slice_items(&self, vm: &VirtualMachine, slice: &PyObjectRef) -> PyResult<Self::Sliced>
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
    type Sliced = Vec<T>;

    fn do_slice(&self, range: Range<usize>) -> Self::Sliced {
        self[range].to_vec()
    }

    fn do_slice_reverse(&self, range: Range<usize>) -> Self::Sliced {
        let mut slice = self[range].to_vec();
        slice.reverse();
        slice
    }

    fn do_stepped_slice(&self, range: Range<usize>, step: usize) -> Self::Sliced {
        self[range].iter().step_by(step).cloned().collect()
    }

    fn do_stepped_slice_reverse(&self, range: Range<usize>, step: usize) -> Self::Sliced {
        self[range].iter().rev().step_by(step).cloned().collect()
    }

    fn empty() -> Self::Sliced {
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
        match_class!(match obj {
            i @ PyInt => Ok(SequenceIndex::Int(i32::try_from_object(
                vm,
                i.into_object()
            )?)),
            s @ PySlice => Ok(SequenceIndex::Slice(s)),
            obj => Err(vm.new_type_error(format!(
                "sequence indices be integers or slices, not {}",
                obj.class(),
            ))),
        })
    }
}

/// Get the index into a sequence like type. Get it from a python integer
/// object, accounting for negative index, and out of bounds issues.
pub fn get_sequence_index(vm: &VirtualMachine, index: &PyIntRef, length: usize) -> PyResult<usize> {
    if let Some(value) = index.as_bigint().to_i64() {
        if value < 0 {
            let from_end: usize = -value as usize;
            if from_end > length {
                Err(vm.new_index_error("Index out of bounds!".to_string()))
            } else {
                let index = length - from_end;
                Ok(index)
            }
        } else {
            let index = value as usize;
            if index >= length {
                Err(vm.new_index_error("Index out of bounds!".to_string()))
            } else {
                Ok(index)
            }
        }
    } else {
        Err(vm.new_index_error("cannot fit 'int' into an index-sized integer".to_string()))
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
            "indexing type {:?} with index {:?} is not supported (yet?)",
            sequence, subscript
        )))
    }
}

type DynPyIter<'a> = Box<dyn ExactSizeIterator<Item = &'a PyObjectRef> + 'a>;

#[allow(clippy::len_without_is_empty)]
pub trait SimpleSeq {
    fn len(&self) -> usize;
    fn iter(&self) -> DynPyIter;
}

impl SimpleSeq for &[PyObjectRef] {
    fn len(&self) -> usize {
        (&**self).len()
    }
    fn iter(&self) -> DynPyIter {
        Box::new((&**self).iter())
    }
}

impl SimpleSeq for std::collections::VecDeque<PyObjectRef> {
    fn len(&self) -> usize {
        self.len()
    }
    fn iter(&self) -> DynPyIter {
        Box::new(self.iter())
    }
}

// impl<'a, I>

pub fn seq_equal(
    vm: &VirtualMachine,
    zelf: &dyn SimpleSeq,
    other: &dyn SimpleSeq,
) -> PyResult<bool> {
    if zelf.len() == other.len() {
        for (a, b) in Iterator::zip(zelf.iter(), other.iter()) {
            if a.is(b) {
                continue;
            }
            if !vm.bool_eq(a.clone(), b.clone())? {
                return Ok(false);
            }
        }
        Ok(true)
    } else {
        Ok(false)
    }
}

pub fn seq_lt(vm: &VirtualMachine, zelf: &dyn SimpleSeq, other: &dyn SimpleSeq) -> PyResult<bool> {
    for (a, b) in Iterator::zip(zelf.iter(), other.iter()) {
        if let Some(v) = vm.bool_seq_lt(a.clone(), b.clone())? {
            return Ok(v);
        }
    }
    Ok(zelf.len() < other.len())
}

pub fn seq_gt(vm: &VirtualMachine, zelf: &dyn SimpleSeq, other: &dyn SimpleSeq) -> PyResult<bool> {
    for (a, b) in Iterator::zip(zelf.iter(), other.iter()) {
        if let Some(v) = vm.bool_seq_gt(a.clone(), b.clone())? {
            return Ok(v);
        }
    }
    Ok(zelf.len() > other.len())
}

pub fn seq_ge(vm: &VirtualMachine, zelf: &dyn SimpleSeq, other: &dyn SimpleSeq) -> PyResult<bool> {
    for (a, b) in Iterator::zip(zelf.iter(), other.iter()) {
        if let Some(v) = vm.bool_seq_gt(a.clone(), b.clone())? {
            return Ok(v);
        }
    }

    Ok(zelf.len() >= other.len())
}

pub fn seq_le(vm: &VirtualMachine, zelf: &dyn SimpleSeq, other: &dyn SimpleSeq) -> PyResult<bool> {
    for (a, b) in Iterator::zip(zelf.iter(), other.iter()) {
        if let Some(v) = vm.bool_seq_lt(a.clone(), b.clone())? {
            return Ok(v);
        }
    }

    Ok(zelf.len() <= other.len())
}

pub struct SeqMul<'a> {
    seq: &'a dyn SimpleSeq,
    repetitions: usize,
    iter: Option<DynPyIter<'a>>,
}
impl<'a> Iterator for SeqMul<'a> {
    type Item = &'a PyObjectRef;
    fn next(&mut self) -> Option<Self::Item> {
        if self.seq.len() == 0 {
            return None;
        }
        match self.iter.as_mut().and_then(Iterator::next) {
            Some(item) => Some(item),
            None => {
                if self.repetitions == 0 {
                    None
                } else {
                    self.repetitions -= 1;
                    self.iter = Some(self.seq.iter());
                    self.next()
                }
            }
        }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let size = self.iter.as_ref().map_or(0, ExactSizeIterator::len)
            + (self.repetitions * self.seq.len());
        (size, Some(size))
    }
}
impl ExactSizeIterator for SeqMul<'_> {}

pub fn seq_mul(seq: &dyn SimpleSeq, repetitions: isize) -> SeqMul {
    SeqMul {
        seq,
        repetitions: repetitions.max(0) as usize,
        iter: None,
    }
}

pub fn get_elements_cell<'a>(obj: &'a PyObjectRef) -> &'a RefCell<Vec<PyObjectRef>> {
    if let Some(list) = obj.payload::<PyList>() {
        return &list.elements;
    }
    panic!("Cannot extract elements from non-sequence");
}

pub fn get_elements_list<'a>(obj: &'a PyObjectRef) -> impl Deref<Target = Vec<PyObjectRef>> + 'a {
    if let Some(list) = obj.payload::<PyList>() {
        return list.elements.borrow();
    }
    panic!("Cannot extract elements from non-sequence");
}

pub fn get_elements_tuple<'a>(obj: &'a PyObjectRef) -> impl Deref<Target = Vec<PyObjectRef>> + 'a {
    if let Some(tuple) = obj.payload::<PyTuple>() {
        return &tuple.elements;
    }
    panic!("Cannot extract elements from non-sequence");
}

pub fn get_mut_elements<'a>(obj: &'a PyObjectRef) -> impl DerefMut<Target = Vec<PyObjectRef>> + 'a {
    if let Some(list) = obj.payload::<PyList>() {
        return list.elements.borrow_mut();
    }
    panic!("Cannot extract elements from non-sequence");
}

//Check if given arg could be used with PySliceableSequence.get_slice_range()
pub fn is_valid_slice_arg(
    arg: OptionalArg<PyObjectRef>,
    vm: &VirtualMachine,
) -> PyResult<Option<BigInt>> {
    if let OptionalArg::Present(value) = arg {
        match_class!(match value {
            i @ PyInt => Ok(Some(i.as_bigint().clone())),
            _obj @ PyNone => Ok(None),
            _ => Err(vm.new_type_error(
                "slice indices must be integers or None or have an __index__ method".to_string()
            )), // TODO: check for an __index__ method
        })
    } else {
        Ok(None)
    }
}
