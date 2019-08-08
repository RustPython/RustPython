use std::cell::{Cell, RefCell};
use std::fmt;

use std::ops::Range;

use num_bigint::{BigInt, ToBigInt};
use num_traits::{One, Signed, ToPrimitive, Zero};

use crate::function::{OptionalArg, PyFuncArgs};
use crate::pyobject::{
    IdProtocol, PyClassImpl, PyContext, PyIterable, PyObjectRef, PyRef, PyResult, PyValue,
    TryFromObject,
};
use crate::vm::{ReprGuard, VirtualMachine};

use super::objbool;
//use super::objint;
use super::objiter;
use super::objsequence::{
    get_elements_cell, get_elements_list, get_item, seq_equal, seq_ge, seq_gt, seq_le, seq_lt,
    seq_mul, SequenceIndex,
};
use super::objslice::PySliceRef;
use super::objtype;
use crate::obj::objtype::PyClassRef;

#[derive(Default)]
pub struct PyList {
    // TODO: shouldn't be public
    pub elements: RefCell<Vec<PyObjectRef>>,
}

impl fmt::Debug for PyList {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // TODO: implement more detailed, non-recursive Debug formatter
        f.write_str("list")
    }
}

impl From<Vec<PyObjectRef>> for PyList {
    fn from(elements: Vec<PyObjectRef>) -> Self {
        PyList {
            elements: RefCell::new(elements),
        }
    }
}

impl PyValue for PyList {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.list_type()
    }
}

impl PyList {
    pub fn get_len(&self) -> usize {
        self.elements.borrow().len()
    }

    pub fn get_pos(&self, p: i32) -> Option<usize> {
        // convert a (potentially negative) positon into a real index
        if p < 0 {
            if -p as usize > self.get_len() {
                None
            } else {
                Some(self.get_len() - ((-p) as usize))
            }
        } else if p as usize >= self.get_len() {
            None
        } else {
            Some(p as usize)
        }
    }

    pub fn get_slice_pos(&self, slice_pos: &BigInt) -> usize {
        if let Some(pos) = slice_pos.to_i32() {
            if let Some(index) = self.get_pos(pos) {
                // within bounds
                return index;
            }
        }

        if slice_pos.is_negative() {
            // slice past start bound, round to start
            0
        } else {
            // slice past end bound, round to end
            self.get_len()
        }
    }

    pub fn get_slice_range(&self, start: &Option<BigInt>, stop: &Option<BigInt>) -> Range<usize> {
        let start = start.as_ref().map(|x| self.get_slice_pos(x)).unwrap_or(0);
        let stop = stop
            .as_ref()
            .map(|x| self.get_slice_pos(x))
            .unwrap_or_else(|| self.get_len());

        start..stop
    }
}

pub type PyListRef = PyRef<PyList>;

impl PyListRef {
    pub fn append(self, x: PyObjectRef, _vm: &VirtualMachine) {
        self.elements.borrow_mut().push(x);
    }

    fn extend(self, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let mut new_elements = vm.extract_elements(&x)?;
        self.elements.borrow_mut().append(&mut new_elements);
        Ok(())
    }

    fn insert(self, position: isize, element: PyObjectRef, _vm: &VirtualMachine) {
        let mut vec = self.elements.borrow_mut();
        let vec_len = vec.len().to_isize().unwrap();
        // This unbounded position can be < 0 or > vec.len()
        let unbounded_position = if position < 0 {
            vec_len + position
        } else {
            position
        };
        // Bound it by [0, vec.len()]
        let position = unbounded_position.max(0).min(vec_len).to_usize().unwrap();
        vec.insert(position, element.clone());
    }

    fn add(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if objtype::isinstance(&other, &vm.ctx.list_type()) {
            let e1 = self.elements.borrow();
            let e2 = get_elements_list(&other);
            let elements = e1.iter().chain(e2.iter()).cloned().collect();
            Ok(vm.ctx.new_list(elements))
        } else {
            Err(vm.new_type_error(format!("Cannot add {} and {}", self.as_object(), other)))
        }
    }

    fn iadd(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if objtype::isinstance(&other, &vm.ctx.list_type()) {
            self.elements
                .borrow_mut()
                .extend_from_slice(&get_elements_list(&other));
            Ok(self.into_object())
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    fn bool(self, _vm: &VirtualMachine) -> bool {
        !self.elements.borrow().is_empty()
    }

    fn clear(self, _vm: &VirtualMachine) {
        self.elements.borrow_mut().clear();
    }

    fn copy(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_list(self.elements.borrow().clone())
    }

    fn len(self, _vm: &VirtualMachine) -> usize {
        self.elements.borrow().len()
    }

    fn reverse(self, _vm: &VirtualMachine) {
        self.elements.borrow_mut().reverse();
    }
    fn reversed(self, _vm: &VirtualMachine) -> PyListReverseIterator {
        let final_position = self.elements.borrow().len();
        PyListReverseIterator {
            position: Cell::new(final_position),
            list: self,
        }
    }

    fn getitem(self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        get_item(
            vm,
            self.as_object(),
            &self.elements.borrow(),
            needle.clone(),
        )
    }

    fn iter(self, _vm: &VirtualMachine) -> PyListIterator {
        PyListIterator {
            position: Cell::new(0),
            list: self,
        }
    }

    fn setitem(
        self,
        subscript: SequenceIndex,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult {
        match subscript {
            SequenceIndex::Int(index) => self.setindex(index, value, vm),
            SequenceIndex::Slice(slice) => {
                if let Ok(sec) = PyIterable::try_from_object(vm, value) {
                    return self.setslice(slice, sec, vm);
                }
                Err(vm.new_type_error("can only assign an iterable to a slice".to_string()))
            }
        }
    }

    fn setindex(self, index: i32, value: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if let Some(pos_index) = self.get_pos(index) {
            self.elements.borrow_mut()[pos_index] = value;
            Ok(vm.get_none())
        } else {
            Err(vm.new_index_error("list assignment index out of range".to_string()))
        }
    }

    fn setslice(self, slice: PySliceRef, sec: PyIterable, vm: &VirtualMachine) -> PyResult {
        let step = slice.step_index(vm)?.unwrap_or_else(BigInt::one);

        if step.is_zero() {
            Err(vm.new_value_error("slice step cannot be zero".to_string()))
        } else if step.is_positive() {
            let range = self.get_slice_range(&slice.start_index(vm)?, &slice.stop_index(vm)?);
            if range.start < range.end {
                match step.to_i32() {
                    Some(1) => self._set_slice(range, sec, vm),
                    Some(num) => {
                        // assign to extended slice
                        self._set_stepped_slice(range, num as usize, sec, vm)
                    }
                    None => {
                        // not sure how this is reached, step too big for i32?
                        // then step is bigger than the than len of the list, no question
                        #[allow(clippy::range_plus_one)]
                        self._set_stepped_slice(range.start..(range.start + 1), 1, sec, vm)
                    }
                }
            } else {
                // this functions as an insert of sec before range.start
                self._set_slice(range.start..range.start, sec, vm)
            }
        } else {
            // calculate the range for the reverse slice, first the bounds needs to be made
            // exclusive around stop, the lower number
            let start = &slice.start_index(vm)?.as_ref().map(|x| {
                if *x == (-1).to_bigint().unwrap() {
                    self.get_len() + BigInt::one() //.to_bigint().unwrap()
                } else {
                    x + 1
                }
            });
            let stop = &slice.stop_index(vm)?.as_ref().map(|x| {
                if *x == (-1).to_bigint().unwrap() {
                    self.get_len().to_bigint().unwrap()
                } else {
                    x + 1
                }
            });
            let range = self.get_slice_range(&stop, &start);
            match (-step).to_i32() {
                Some(num) => self._set_stepped_slice_reverse(range, num as usize, sec, vm),
                None => {
                    // not sure how this is reached, step too big for i32?
                    // then step is bigger than the than len of the list no question
                    self._set_stepped_slice_reverse(range.end - 1..range.end, 1, sec, vm)
                }
            }
        }
    }

    fn _set_slice(self, range: Range<usize>, sec: PyIterable, vm: &VirtualMachine) -> PyResult {
        // consume the iter, we  need it's size
        // and if it's going to fail we want that to happen *before* we start modifing
        let items: Result<Vec<PyObjectRef>, _> = sec.iter(vm)?.collect();
        let items = items?;

        // replace the range of elements with the full sequence
        self.elements.borrow_mut().splice(range, items);

        Ok(vm.get_none())
    }

    fn _set_stepped_slice(
        self,
        range: Range<usize>,
        step: usize,
        sec: PyIterable,
        vm: &VirtualMachine,
    ) -> PyResult {
        let slicelen = if range.end > range.start {
            ((range.end - range.start - 1) / step) + 1
        } else {
            0
        };
        // consume the iter, we  need it's size
        // and if it's going to fail we want that to happen *before* we start modifing
        let items: Result<Vec<PyObjectRef>, _> = sec.iter(vm)?.collect();
        let items = items?;

        let n = items.len();

        if range.start < range.end {
            if n == slicelen {
                let indexes = range.step_by(step);
                self._replace_indexes(indexes, &items);
                Ok(vm.get_none())
            } else {
                Err(vm.new_value_error(format!(
                    "attempt to assign sequence of size {} to extended slice of size {}",
                    n, slicelen
                )))
            }
        } else if n == 0 {
            // slice is empty but so is sequence
            Ok(vm.get_none())
        } else {
            // empty slice but this is an error because stepped slice
            Err(vm.new_value_error(format!(
                "attempt to assign sequence of size {} to extended slice of size 0",
                n
            )))
        }
    }

    fn _set_stepped_slice_reverse(
        self,
        range: Range<usize>,
        step: usize,
        sec: PyIterable,
        vm: &VirtualMachine,
    ) -> PyResult {
        let slicelen = if range.end > range.start {
            ((range.end - range.start - 1) / step) + 1
        } else {
            0
        };

        // consume the iter, we  need it's size
        // and if it's going to fail we want that to happen *before* we start modifing
        let items: Result<Vec<PyObjectRef>, _> = sec.iter(vm)?.collect();
        let items = items?;

        let n = items.len();

        if range.start < range.end {
            if n == slicelen {
                let indexes = range.rev().step_by(step);
                self._replace_indexes(indexes, &items);
                Ok(vm.get_none())
            } else {
                Err(vm.new_value_error(format!(
                    "attempt to assign sequence of size {} to extended slice of size {}",
                    n, slicelen
                )))
            }
        } else if n == 0 {
            // slice is empty but so is sequence
            Ok(vm.get_none())
        } else {
            // empty slice but this is an error because stepped slice
            Err(vm.new_value_error(format!(
                "attempt to assign sequence of size {} to extended slice of size 0",
                n
            )))
        }
    }

    fn _replace_indexes<I>(self, indexes: I, items: &[PyObjectRef])
    where
        I: Iterator<Item = usize>,
    {
        let mut elements = self.elements.borrow_mut();

        for (i, value) in indexes.zip(items) {
            // clone for refrence count
            elements[i] = value.clone();
        }
    }

    fn repr(self, vm: &VirtualMachine) -> PyResult<String> {
        let s = if let Some(_guard) = ReprGuard::enter(self.as_object()) {
            let mut str_parts = vec![];
            for elem in self.elements.borrow().iter() {
                let s = vm.to_repr(elem)?;
                str_parts.push(s.value.clone());
            }
            format!("[{}]", str_parts.join(", "))
        } else {
            "[...]".to_string()
        };
        Ok(s)
    }

    fn hash(self, vm: &VirtualMachine) -> PyResult<()> {
        Err(vm.new_type_error("unhashable type".to_string()))
    }

    fn mul(self, counter: isize, vm: &VirtualMachine) -> PyObjectRef {
        let new_elements = seq_mul(&self.elements.borrow().as_slice(), counter)
            .cloned()
            .collect();
        vm.ctx.new_list(new_elements)
    }

    fn rmul(self, counter: isize, vm: &VirtualMachine) -> PyObjectRef {
        self.mul(counter, &vm)
    }

    fn imul(self, counter: isize, _vm: &VirtualMachine) -> Self {
        let new_elements = seq_mul(&self.elements.borrow().as_slice(), counter)
            .cloned()
            .collect();
        self.elements.replace(new_elements);
        self
    }

    fn count(self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
        let mut count: usize = 0;
        for element in self.elements.borrow().iter() {
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
        for element in self.elements.borrow().iter() {
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
        for (index, element) in self.elements.borrow().iter().enumerate() {
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

    fn pop(self, i: OptionalArg<isize>, vm: &VirtualMachine) -> PyResult {
        let mut i = i.into_option().unwrap_or(-1);
        let mut elements = self.elements.borrow_mut();
        if i < 0 {
            i += elements.len() as isize;
        }
        if elements.is_empty() {
            Err(vm.new_index_error("pop from empty list".to_string()))
        } else if i < 0 || i as usize >= elements.len() {
            Err(vm.new_index_error("pop index out of range".to_string()))
        } else {
            Ok(elements.remove(i as usize))
        }
    }

    fn remove(self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let mut ri: Option<usize> = None;
        for (index, element) in self.elements.borrow().iter().enumerate() {
            if needle.is(element) {
                ri = Some(index);
                break;
            }
            let py_equal = vm._eq(needle.clone(), element.clone())?;
            if objbool::get_value(&py_equal) {
                ri = Some(index);
                break;
            }
        }

        if let Some(index) = ri {
            self.elements.borrow_mut().remove(index);
            Ok(())
        } else {
            let needle_str = &vm.to_str(&needle)?.value;
            Err(vm.new_value_error(format!("'{}' is not in list", needle_str)))
        }
    }

    fn eq(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if self.as_object().is(&other) {
            return Ok(vm.new_bool(true));
        }

        if objtype::isinstance(&other, &vm.ctx.list_type()) {
            let zelf = self.elements.borrow();
            let other = get_elements_list(&other);
            let res = seq_equal(vm, &zelf.as_slice(), &other.as_slice())?;
            Ok(vm.new_bool(res))
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    fn lt(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if objtype::isinstance(&other, &vm.ctx.list_type()) {
            let zelf = self.elements.borrow();
            let other = get_elements_list(&other);
            let res = seq_lt(vm, &zelf.as_slice(), &other.as_slice())?;
            Ok(vm.new_bool(res))
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    fn gt(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if objtype::isinstance(&other, &vm.ctx.list_type()) {
            let zelf = self.elements.borrow();
            let other = get_elements_list(&other);
            let res = seq_gt(vm, &zelf.as_slice(), &other.as_slice())?;
            Ok(vm.new_bool(res))
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    fn ge(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if objtype::isinstance(&other, &vm.ctx.list_type()) {
            let zelf = self.elements.borrow();
            let other = get_elements_list(&other);
            let res = seq_ge(vm, &zelf.as_slice(), &other.as_slice())?;
            Ok(vm.new_bool(res))
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    fn le(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if objtype::isinstance(&other, &vm.ctx.list_type()) {
            let zelf = self.elements.borrow();
            let other = get_elements_list(&other);
            let res = seq_le(vm, &zelf.as_slice(), &other.as_slice())?;
            Ok(vm.new_bool(res))
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    fn delitem(self, subscript: SequenceIndex, vm: &VirtualMachine) -> PyResult {
        match subscript {
            SequenceIndex::Int(index) => self.delindex(index, vm),
            SequenceIndex::Slice(slice) => self.delslice(slice, vm),
        }
    }

    fn delindex(self, index: i32, vm: &VirtualMachine) -> PyResult {
        if let Some(pos_index) = self.get_pos(index) {
            self.elements.borrow_mut().remove(pos_index);
            Ok(vm.get_none())
        } else {
            Err(vm.new_index_error("Index out of bounds!".to_string()))
        }
    }

    fn delslice(self, slice: PySliceRef, vm: &VirtualMachine) -> PyResult {
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
                    Some(1) => {
                        self._del_slice(range);
                        Ok(vm.get_none())
                    }
                    Some(num) => {
                        self._del_stepped_slice(range, num as usize);
                        Ok(vm.get_none())
                    }
                    None => {
                        self._del_slice(range.start..range.start + 1);
                        Ok(vm.get_none())
                    }
                }
            } else {
                // no del to do
                Ok(vm.get_none())
            }
        } else {
            // calculate the range for the reverse slice, first the bounds needs to be made
            // exclusive around stop, the lower number
            let start = start.as_ref().map(|x| {
                if *x == (-1).to_bigint().unwrap() {
                    self.get_len() + BigInt::one() //.to_bigint().unwrap()
                } else {
                    x + 1
                }
            });
            let stop = stop.as_ref().map(|x| {
                if *x == (-1).to_bigint().unwrap() {
                    self.get_len().to_bigint().unwrap()
                } else {
                    x + 1
                }
            });
            let range = self.get_slice_range(&stop, &start);
            if range.start < range.end {
                match (-step).to_i32() {
                    Some(1) => {
                        self._del_slice(range);
                        Ok(vm.get_none())
                    }
                    Some(num) => {
                        self._del_stepped_slice_reverse(range, num as usize);
                        Ok(vm.get_none())
                    }
                    None => {
                        self._del_slice(range.end - 1..range.end);
                        Ok(vm.get_none())
                    }
                }
            } else {
                // no del to do
                Ok(vm.get_none())
            }
        }
    }

    fn _del_slice(self, range: Range<usize>) {
        self.elements.borrow_mut().drain(range);
    }

    fn _del_stepped_slice(self, range: Range<usize>, step: usize) {
        // no easy way to delete stepped indexes so here is what we'll do
        let mut deleted = 0;
        let mut elements = self.elements.borrow_mut();
        let mut indexes = range.clone().step_by(step).peekable();

        for i in range.clone() {
            // is this an index to delete?
            if indexes.peek() == Some(&i) {
                // record and move on
                indexes.next();
                deleted += 1;
            } else {
                // swap towards front
                elements.swap(i - deleted, i);
            }
        }
        // then drain (the values to delete should now be contiguous at the end of the range)
        elements.drain((range.end - deleted)..range.end);
    }

    fn _del_stepped_slice_reverse(self, range: Range<usize>, step: usize) {
        // no easy way to delete stepped indexes so here is what we'll do
        let mut deleted = 0;
        let mut elements = self.elements.borrow_mut();
        let mut indexes = range.clone().rev().step_by(step).peekable();

        for i in range.clone().rev() {
            // is this an index to delete?
            if indexes.peek() == Some(&i) {
                // record and move on
                indexes.next();
                deleted += 1;
            } else {
                // swap towards back
                elements.swap(i + deleted, i);
            }
        }
        // then drain (the values to delete should now be contiguous at teh start of the range)
        elements.drain(range.start..(range.start + deleted));
    }
}

fn list_new(
    cls: PyClassRef,
    iterable: OptionalArg<PyObjectRef>,
    vm: &VirtualMachine,
) -> PyResult<PyListRef> {
    let elements = if let OptionalArg::Present(iterable) = iterable {
        vm.extract_elements(&iterable)?
    } else {
        vec![]
    };

    PyList::from(elements).into_ref_with_type(vm, cls)
}

fn quicksort(
    vm: &VirtualMachine,
    keys: &mut [PyObjectRef],
    values: &mut [PyObjectRef],
) -> PyResult<()> {
    let len = values.len();
    if len >= 2 {
        let pivot = partition(vm, keys, values)?;
        quicksort(vm, &mut keys[0..pivot], &mut values[0..pivot])?;
        quicksort(vm, &mut keys[pivot + 1..len], &mut values[pivot + 1..len])?;
    }
    Ok(())
}

fn partition(
    vm: &VirtualMachine,
    keys: &mut [PyObjectRef],
    values: &mut [PyObjectRef],
) -> PyResult<usize> {
    let len = values.len();
    let pivot = len / 2;

    values.swap(pivot, len - 1);
    keys.swap(pivot, len - 1);

    let mut store_idx = 0;
    for i in 0..len - 1 {
        let result = vm._lt(keys[i].clone(), keys[len - 1].clone())?;
        let boolval = objbool::boolval(vm, result)?;
        if boolval {
            values.swap(i, store_idx);
            keys.swap(i, store_idx);
            store_idx += 1;
        }
    }

    values.swap(store_idx, len - 1);
    keys.swap(store_idx, len - 1);
    Ok(store_idx)
}

fn do_sort(
    vm: &VirtualMachine,
    values: &mut Vec<PyObjectRef>,
    key_func: Option<PyObjectRef>,
    reverse: bool,
) -> PyResult<()> {
    // build a list of keys. If no keyfunc is provided, it's a copy of the list.
    let mut keys: Vec<PyObjectRef> = vec![];
    for x in values.iter() {
        keys.push(match &key_func {
            None => x.clone(),
            Some(ref func) => vm.invoke((*func).clone(), vec![x.clone()])?,
        });
    }

    quicksort(vm, &mut keys, values)?;

    if reverse {
        values.reverse();
    }

    Ok(())
}

fn list_sort(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(list, Some(vm.ctx.list_type()))]);
    let key_func = args.get_optional_kwarg("key");
    let reverse = args.get_optional_kwarg("reverse");
    let reverse = match reverse {
        None => false,
        Some(val) => objbool::boolval(vm, val)?,
    };

    let elements_cell = get_elements_cell(list);
    // replace list contents with [] for duration of sort.
    // this prevents keyfunc from messing with the list and makes it easy to
    // check if it tries to append elements to it.
    let mut elements = elements_cell.replace(vec![]);
    do_sort(vm, &mut elements, key_func, reverse)?;
    let temp_elements = elements_cell.replace(elements);

    if !temp_elements.is_empty() {
        return Err(vm.new_value_error("list modified during sort".to_string()));
    }

    Ok(vm.get_none())
}

#[pyclass]
#[derive(Debug)]
pub struct PyListIterator {
    pub position: Cell<usize>,
    pub list: PyListRef,
}

impl PyValue for PyListIterator {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.listiterator_type()
    }
}

#[pyimpl]
impl PyListIterator {
    #[pymethod(name = "__next__")]
    fn next(&self, vm: &VirtualMachine) -> PyResult {
        if self.position.get() < self.list.elements.borrow().len() {
            let ret = self.list.elements.borrow()[self.position.get()].clone();
            self.position.set(self.position.get() + 1);
            Ok(ret)
        } else {
            Err(objiter::new_stop_iteration(vm))
        }
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyRef<Self> {
        zelf
    }
}

#[pyclass]
#[derive(Debug)]
pub struct PyListReverseIterator {
    pub position: Cell<usize>,
    pub list: PyListRef,
}

impl PyValue for PyListReverseIterator {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.listreverseiterator_type()
    }
}

#[pyimpl]
impl PyListReverseIterator {
    #[pymethod(name = "__next__")]
    fn next(&self, vm: &VirtualMachine) -> PyResult {
        if self.position.get() > 0 {
            let position: usize = self.position.get() - 1;
            let ret = self.list.elements.borrow()[position].clone();
            self.position.set(position);
            Ok(ret)
        } else {
            Err(objiter::new_stop_iteration(vm))
        }
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyRef<Self> {
        zelf
    }
}

#[rustfmt::skip] // to avoid line splitting
pub fn init(context: &PyContext) {
    let list_type = &context.list_type;

    let list_doc = "Built-in mutable sequence.\n\n\
                    If no argument is given, the constructor creates a new empty list.\n\
                    The argument must be an iterable if specified.";

    extend_class!(context, list_type, {
        "__add__" => context.new_rustfunc(PyListRef::add),
        "__iadd__" => context.new_rustfunc(PyListRef::iadd),
        "__bool__" => context.new_rustfunc(PyListRef::bool),
        "__contains__" => context.new_rustfunc(PyListRef::contains),
        "__delitem__" => context.new_rustfunc(PyListRef::delitem),
        "__eq__" => context.new_rustfunc(PyListRef::eq),
        "__lt__" => context.new_rustfunc(PyListRef::lt),
        "__gt__" => context.new_rustfunc(PyListRef::gt),
        "__le__" => context.new_rustfunc(PyListRef::le),
        "__ge__" => context.new_rustfunc(PyListRef::ge),
        "__getitem__" => context.new_rustfunc(PyListRef::getitem),
        "__iter__" => context.new_rustfunc(PyListRef::iter),
        "__setitem__" => context.new_rustfunc(PyListRef::setitem),
        "__reversed__" => context.new_rustfunc(PyListRef::reversed),
        "__mul__" => context.new_rustfunc(PyListRef::mul),
        "__rmul__" => context.new_rustfunc(PyListRef::rmul),
        "__imul__" => context.new_rustfunc(PyListRef::imul),
        "__len__" => context.new_rustfunc(PyListRef::len),
        "__new__" => context.new_rustfunc(list_new),
        "__repr__" => context.new_rustfunc(PyListRef::repr),
        "__hash__" => context.new_rustfunc(PyListRef::hash),
        "__doc__" => context.new_str(list_doc.to_string()),
        "append" => context.new_rustfunc(PyListRef::append),
        "clear" => context.new_rustfunc(PyListRef::clear),
        "copy" => context.new_rustfunc(PyListRef::copy),
        "count" => context.new_rustfunc(PyListRef::count),
        "extend" => context.new_rustfunc(PyListRef::extend),
        "index" => context.new_rustfunc(PyListRef::index),
        "insert" => context.new_rustfunc(PyListRef::insert),
        "reverse" => context.new_rustfunc(PyListRef::reverse),
        "sort" => context.new_rustfunc(list_sort),
        "pop" => context.new_rustfunc(PyListRef::pop),
        "remove" => context.new_rustfunc(PyListRef::remove)
    });

    PyListIterator::extend_class(context, &context.listiterator_type);
    PyListReverseIterator::extend_class(context, &context.listreverseiterator_type);
}
