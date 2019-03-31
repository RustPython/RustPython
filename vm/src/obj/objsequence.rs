use std::cell::{Cell, RefCell};
use std::marker::Sized;
use std::ops::{Deref, DerefMut, Range};

use num_bigint::BigInt;
use num_traits::{One, Signed, ToPrimitive, Zero};

use crate::pyobject::{IdProtocol, PyIteratorValue, PyObject, PyObjectRef, PyResult, TypeProtocol};
use crate::vm::VirtualMachine;

use super::objbool;
use super::objint::PyInt;
use super::objlist::PyList;
use super::objslice::PySlice;
use super::objtuple::PyTuple;
use super::objtype;
use super::objtype::PyClassRef;

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
        // TODO: we could potentially avoid this copy and use slice
        match slice.payload() {
            Some(PySlice { start, stop, step }) => {
                let step = step.clone().unwrap_or_else(BigInt::one);
                if step.is_zero() {
                    Err(vm.new_value_error("slice step cannot be zero".to_string()))
                } else if step.is_positive() {
                    let range = self.get_slice_range(start, stop);
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

pub trait SequenceProtocol
where
    Self: Sized,
{
    fn get_elements(&self) -> Vec<PyObjectRef>;
    fn as_object(&self) -> &PyObjectRef;
    fn into_object(self) -> PyObjectRef;
    fn create(&self, vm: &VirtualMachine, elements: Vec<PyObjectRef>) -> PyResult;
    fn class(&self) -> PyClassRef;

    fn bool(self, _vm: &VirtualMachine) -> bool {
        !self.get_elements().is_empty()
    }

    fn copy(self, vm: &VirtualMachine) -> PyResult {
        self.create(vm, self.get_elements())
    }

    fn len(self, _vm: &VirtualMachine) -> usize {
        self.get_elements().len()
    }

    fn getitem(self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        get_item(vm, self.as_object(), &self.get_elements(), needle.clone())
    }

    fn iter(self, _vm: &VirtualMachine) -> PyIteratorValue {
        PyIteratorValue {
            position: Cell::new(0),
            iterated_obj: self.into_object(),
        }
    }

    fn mul(self, counter: isize, vm: &VirtualMachine) -> PyResult {
        let elements = self.get_elements();
        let current_len = elements.len();
        let new_len = counter.max(0) as usize * current_len;
        let mut new_elements = Vec::with_capacity(new_len);

        for _ in 0..counter {
            new_elements.extend(elements.to_owned());
        }

        self.create(vm, new_elements)
    }

    fn count(self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
        let mut count: usize = 0;
        for element in self.get_elements().iter() {
            if needle.is(element) {
                count += 1;
            } else {
                let py_equal = vm._eq(element.clone(), needle.clone())?;
                if objbool::boolval(vm, py_equal)? {
                    count += 1;
                }
            }
        }
        Ok(count)
    }

    fn contains(self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        for element in self.get_elements().iter() {
            if needle.is(element) {
                return Ok(true);
            }
            let py_equal = vm._eq(element.clone(), needle.clone())?;
            if objbool::boolval(vm, py_equal)? {
                return Ok(true);
            }
        }

        Ok(false)
    }

    fn index(self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
        for (index, element) in self.get_elements().iter().enumerate() {
            if needle.is(element) {
                return Ok(index);
            }
            let py_equal = vm._eq(needle.clone(), element.clone())?;
            if objbool::boolval(vm, py_equal)? {
                return Ok(index);
            }
        }
        let needle_str = &vm.to_str(&needle)?.value;
        Err(vm.new_value_error(format!("'{}' is not in list", needle_str)))
    }

    fn eq(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if self.as_object().is(&other) {
            return Ok(vm.new_bool(true));
        }

        if objtype::isinstance(&other, &self.class()) {
            let zelf = self.get_elements();
            let other = get_elements(&other);
            let res = seq_equal(vm, &zelf, &other)?;
            Ok(vm.new_bool(res))
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    fn lt(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if objtype::isinstance(&other, &self.class()) {
            let zelf = self.get_elements();
            let other = get_elements(&other);
            let res = seq_lt(vm, &zelf, &other)?;
            Ok(vm.new_bool(res))
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    fn gt(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if objtype::isinstance(&other, &self.class()) {
            let zelf = self.get_elements();
            let other = get_elements(&other);
            let res = seq_gt(vm, &zelf, &other)?;
            Ok(vm.new_bool(res))
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    fn ge(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if objtype::isinstance(&other, &self.class()) {
            let zelf = &self.get_elements();
            let other = &get_elements(&other);
            Ok(vm.new_bool(seq_gt(vm, zelf, other)? || seq_equal(vm, zelf, other)?))
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    fn le(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if objtype::isinstance(&other, &self.class()) {
            let zelf = &self.get_elements();
            let other = &get_elements(&other);
            Ok(vm.new_bool(seq_lt(vm, zelf, other)? || seq_equal(vm, zelf, other)?))
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }
}

fn get_item(
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

fn seq_equal(
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

fn seq_lt(
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

fn seq_gt(
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
