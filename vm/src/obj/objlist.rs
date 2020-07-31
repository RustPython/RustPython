use std::fmt;
use std::iter::FromIterator;
use std::mem::size_of;
use std::ops::{DerefMut, Range};

use crossbeam_utils::atomic::AtomicCell;
use num_bigint::{BigInt, ToBigInt};
use num_traits::{One, Signed, ToPrimitive, Zero};

use super::objbool;
use super::objint::PyIntRef;
use super::objiter;
use super::objsequence::{get_item, get_pos, get_slice_range, SequenceIndex};
use super::objslice::PySliceRef;
use super::objtype::PyClassRef;
use crate::bytesinner;
use crate::common::cell::{PyRwLock, PyRwLockReadGuard, PyRwLockWriteGuard};
use crate::function::OptionalArg;
use crate::pyobject::{
    IdProtocol, PyArithmaticValue::*, PyClassImpl, PyComparisonValue, PyContext, PyIterable,
    PyObjectRef, PyRef, PyResult, PyValue, TryFromObject, TypeProtocol,
};
use crate::sequence::{self, SimpleSeq};
use crate::vm::{ReprGuard, VirtualMachine};

/// Built-in mutable sequence.
///
/// If no argument is given, the constructor creates a new empty list.
/// The argument must be an iterable if specified.
#[pyclass]
#[derive(Default)]
pub struct PyList {
    elements: PyRwLock<Vec<PyObjectRef>>,
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
            elements: PyRwLock::new(elements),
        }
    }
}

impl FromIterator<PyObjectRef> for PyList {
    fn from_iter<T: IntoIterator<Item = PyObjectRef>>(iter: T) -> Self {
        Vec::from_iter(iter).into()
    }
}

impl PyValue for PyList {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.list_type()
    }
}

impl PyList {
    pub fn borrow_elements(&self) -> PyRwLockReadGuard<'_, Vec<PyObjectRef>> {
        self.elements.read()
    }

    pub fn borrow_elements_mut(&self) -> PyRwLockWriteGuard<'_, Vec<PyObjectRef>> {
        self.elements.write()
    }

    pub(crate) fn to_byte_inner(&self, vm: &VirtualMachine) -> PyResult<bytesinner::PyBytesInner> {
        let mut elements = Vec::<u8>::with_capacity(self.borrow_elements().len());
        for elem in self.borrow_elements().iter() {
            match PyIntRef::try_from_object(vm, elem.clone()) {
                Ok(result) => match result.as_bigint().to_u8() {
                    Some(result) => elements.push(result),
                    None => {
                        return Err(vm.new_value_error("bytes must be in range (0, 256)".to_owned()))
                    }
                },
                _ => {
                    return Err(vm.new_type_error(format!(
                        "'{}' object cannot be interpreted as an integer",
                        elem.class().name
                    )))
                }
            }
        }
        Ok(bytesinner::PyBytesInner { elements })
    }
}

#[derive(FromArgs)]
struct SortOptions {
    #[pyarg(keyword_only, default = "None")]
    key: Option<PyObjectRef>,
    #[pyarg(keyword_only, default = "false")]
    reverse: bool,
}

pub type PyListRef = PyRef<PyList>;

#[pyimpl(flags(BASETYPE))]
impl PyList {
    #[pymethod]
    pub(crate) fn append(&self, x: PyObjectRef) {
        self.borrow_elements_mut().push(x);
    }

    #[pymethod]
    fn extend(&self, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let mut new_elements = vm.extract_elements(&x)?;
        self.borrow_elements_mut().append(&mut new_elements);
        Ok(())
    }

    #[pymethod]
    pub(crate) fn insert(&self, position: isize, element: PyObjectRef) {
        let mut elements = self.borrow_elements_mut();
        let vec_len = elements.len().to_isize().unwrap();
        // This unbounded position can be < 0 or > vec.len()
        let unbounded_position = if position < 0 {
            vec_len + position
        } else {
            position
        };
        // Bound it by [0, vec.len()]
        let position = unbounded_position.min(vec_len).to_usize().unwrap_or(0);
        elements.insert(position, element.clone());
    }

    #[pymethod(name = "__add__")]
    fn add(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if let Some(other) = other.payload_if_subclass::<PyList>(vm) {
            let e1 = self.borrow_elements().clone();
            let e2 = other.borrow_elements().clone();
            let elements = e1.iter().chain(e2.iter()).cloned().collect();
            Ok(vm.ctx.new_list(elements))
        } else {
            Err(vm.new_type_error(format!(
                "Cannot add {} and {}",
                Self::class(vm).name,
                other.lease_class().name
            )))
        }
    }

    #[pymethod(name = "__iadd__")]
    fn iadd(zelf: PyRef<Self>, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if let Ok(new_elements) = vm.extract_elements(&other) {
            let mut e = new_elements;
            zelf.borrow_elements_mut().append(&mut e);
            Ok(zelf.into_object())
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    #[pymethod(name = "__bool__")]
    fn bool(&self) -> bool {
        !self.borrow_elements().is_empty()
    }

    #[pymethod]
    fn clear(&self) {
        self.borrow_elements_mut().clear();
    }

    #[pymethod]
    fn copy(&self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_list(self.borrow_elements().clone())
    }

    #[pymethod(name = "__len__")]
    fn len(&self) -> usize {
        self.borrow_elements().len()
    }

    #[pymethod(name = "__sizeof__")]
    fn sizeof(&self) -> usize {
        size_of::<Self>() + self.borrow_elements().capacity() * size_of::<PyObjectRef>()
    }

    #[pymethod]
    fn reverse(&self) {
        self.borrow_elements_mut().reverse();
    }

    #[pymethod(name = "__reversed__")]
    fn reversed(zelf: PyRef<Self>) -> PyListReverseIterator {
        let final_position = zelf.borrow_elements().len();
        PyListReverseIterator {
            position: AtomicCell::new(final_position as isize),
            list: zelf,
        }
    }

    #[pymethod(name = "__getitem__")]
    fn getitem(zelf: PyRef<Self>, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        get_item(
            vm,
            zelf.as_object(),
            &zelf.borrow_elements(),
            needle.clone(),
        )
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>) -> PyListIterator {
        PyListIterator {
            position: AtomicCell::new(0),
            list: zelf,
        }
    }

    #[pymethod(name = "__setitem__")]
    fn setitem(
        &self,
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
                Err(vm.new_type_error("can only assign an iterable to a slice".to_owned()))
            }
        }
    }

    fn setindex(&self, index: isize, value: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let mut elements = self.borrow_elements_mut();
        if let Some(pos_index) = get_pos(index, elements.len()) {
            elements[pos_index] = value;
            Ok(vm.get_none())
        } else {
            Err(vm.new_index_error("list assignment index out of range".to_owned()))
        }
    }

    fn setslice(&self, slice: PySliceRef, sec: PyIterable, vm: &VirtualMachine) -> PyResult {
        let step = slice.step_index(vm)?.unwrap_or_else(BigInt::one);
        let start = slice.start_index(vm)?;
        let stop = slice.stop_index(vm)?;
        // consume the iter, we  need it's size
        // and if it's going to fail we want that to happen *before* we start modifing
        // In addition we want to iterate before taking the list lock.
        let items: Result<Vec<PyObjectRef>, _> = sec.iter(vm)?.collect();
        let items = items?;

        let elements = self.borrow_elements_mut();
        if step.is_zero() {
            Err(vm.new_value_error("slice step cannot be zero".to_owned()))
        } else if step.is_positive() {
            let range = get_slice_range(&start, &stop, elements.len());
            if range.start < range.end {
                match step.to_isize() {
                    Some(1) => PyList::_set_slice(elements, range, items, vm),
                    Some(num) => {
                        // assign to extended slice
                        PyList::_set_stepped_slice(elements, range, num as usize, items, vm)
                    }
                    None => {
                        // not sure how this is reached, step too big for isize?
                        // then step is bigger than the than len of the list, no question
                        #[allow(clippy::range_plus_one)]
                        PyList::_set_stepped_slice(
                            elements,
                            range.start..(range.start + 1),
                            1,
                            items,
                            vm,
                        )
                    }
                }
            } else {
                // this functions as an insert of sec before range.start
                PyList::_set_slice(elements, range.start..range.start, items, vm)
            }
        } else {
            // calculate the range for the reverse slice, first the bounds needs to be made
            // exclusive around stop, the lower number
            let start = &start.as_ref().map(|x| {
                if *x == (-1).to_bigint().unwrap() {
                    elements.len() + BigInt::one() //.to_bigint().unwrap()
                } else {
                    x + 1
                }
            });
            let stop = &stop.as_ref().map(|x| {
                if *x == (-1).to_bigint().unwrap() {
                    elements.len().to_bigint().unwrap()
                } else {
                    x + 1
                }
            });
            let range = get_slice_range(&stop, &start, elements.len());
            match (-step).to_isize() {
                Some(num) => {
                    PyList::_set_stepped_slice_reverse(elements, range, num as usize, items, vm)
                }
                None => {
                    // not sure how this is reached, step too big for isize?
                    // then step is bigger than the than len of the list no question
                    PyList::_set_stepped_slice_reverse(
                        elements,
                        range.end - 1..range.end,
                        1,
                        items,
                        vm,
                    )
                }
            }
        }
    }

    fn _set_slice(
        mut elements: PyRwLockWriteGuard<'_, Vec<PyObjectRef>>,
        range: Range<usize>,
        items: Vec<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        // replace the range of elements with the full sequence
        elements.splice(range, items);

        Ok(vm.get_none())
    }

    fn _set_stepped_slice(
        elements: PyRwLockWriteGuard<'_, Vec<PyObjectRef>>,
        range: Range<usize>,
        step: usize,
        items: Vec<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let slicelen = if range.end > range.start {
            ((range.end - range.start - 1) / step) + 1
        } else {
            0
        };

        let n = items.len();

        if range.start < range.end {
            if n == slicelen {
                let indexes = range.step_by(step);
                PyList::_replace_indexes(elements, indexes, &items);
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
        elements: PyRwLockWriteGuard<'_, Vec<PyObjectRef>>,
        range: Range<usize>,
        step: usize,
        items: Vec<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let slicelen = if range.end > range.start {
            ((range.end - range.start - 1) / step) + 1
        } else {
            0
        };

        let n = items.len();

        if range.start < range.end {
            if n == slicelen {
                let indexes = range.rev().step_by(step);
                PyList::_replace_indexes(elements, indexes, &items);
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

    fn _replace_indexes<I>(
        mut elements: PyRwLockWriteGuard<'_, Vec<PyObjectRef>>,
        indexes: I,
        items: &[PyObjectRef],
    ) where
        I: Iterator<Item = usize>,
    {
        for (i, value) in indexes.zip(items) {
            // clone for refrence count
            elements[i] = value.clone();
        }
    }

    #[pymethod(name = "__repr__")]
    fn repr(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<String> {
        let s = if let Some(_guard) = ReprGuard::enter(zelf.as_object()) {
            let elements = zelf.borrow_elements().clone();
            let mut str_parts = Vec::with_capacity(elements.len());
            for elem in elements.iter() {
                let s = vm.to_repr(elem)?;
                str_parts.push(s.as_str().to_owned());
            }
            format!("[{}]", str_parts.join(", "))
        } else {
            "[...]".to_owned()
        };
        Ok(s)
    }

    #[pymethod(name = "__hash__")]
    fn hash(&self, vm: &VirtualMachine) -> PyResult<()> {
        Err(vm.new_type_error("unhashable type".to_owned()))
    }

    #[pymethod(name = "__mul__")]
    fn mul(&self, counter: isize, vm: &VirtualMachine) -> PyObjectRef {
        let new_elements = sequence::seq_mul(&*self.borrow_elements(), counter)
            .cloned()
            .collect();
        vm.ctx.new_list(new_elements)
    }

    #[pymethod(name = "__rmul__")]
    fn rmul(&self, counter: isize, vm: &VirtualMachine) -> PyObjectRef {
        self.mul(counter, &vm)
    }

    #[pymethod(name = "__imul__")]
    fn imul(zelf: PyRef<Self>, counter: isize) -> PyRef<Self> {
        let mut elements = zelf.borrow_elements_mut();
        let mut new_elements: Vec<PyObjectRef> =
            sequence::seq_mul(&*elements, counter).cloned().collect();
        std::mem::swap(elements.deref_mut(), &mut new_elements);
        zelf.clone()
    }

    #[pymethod]
    fn count(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
        let mut count: usize = 0;
        for element in self.borrow_elements().clone().iter() {
            if vm.identical_or_equal(element, &needle)? {
                count += 1;
            }
        }
        Ok(count)
    }

    #[pymethod(name = "__contains__")]
    fn contains(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        for element in self.borrow_elements().clone().iter() {
            if vm.identical_or_equal(element, &needle)? {
                return Ok(true);
            }
        }

        Ok(false)
    }

    #[pymethod]
    fn index(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
        for (index, element) in self.borrow_elements().clone().iter().enumerate() {
            if vm.identical_or_equal(element, &needle)? {
                return Ok(index);
            }
        }
        let needle_str = vm.to_str(&needle)?;
        Err(vm.new_value_error(format!("'{}' is not in list", needle_str.as_str())))
    }

    #[pymethod]
    fn pop(&self, i: OptionalArg<isize>, vm: &VirtualMachine) -> PyResult {
        let mut i = i.into_option().unwrap_or(-1);
        let mut elements = self.borrow_elements_mut();
        if i < 0 {
            i += elements.len() as isize;
        }
        if elements.is_empty() {
            Err(vm.new_index_error("pop from empty list".to_owned()))
        } else if i < 0 || i as usize >= elements.len() {
            Err(vm.new_index_error("pop index out of range".to_owned()))
        } else {
            Ok(elements.remove(i as usize))
        }
    }

    #[pymethod]
    fn remove(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let mut ri: Option<usize> = None;
        for (index, element) in self.borrow_elements().clone().iter().enumerate() {
            if vm.identical_or_equal(element, &needle)? {
                ri = Some(index);
                break;
            }
        }

        if let Some(index) = ri {
            // TODO: Check if value was removed after lock released
            self.borrow_elements_mut().remove(index);
            Ok(())
        } else {
            let needle_str = vm.to_str(&needle)?;
            Err(vm.new_value_error(format!("'{}' is not in list", needle_str.as_str())))
        }
    }

    #[inline]
    fn cmp<F>(&self, other: PyObjectRef, op: F, vm: &VirtualMachine) -> PyResult<PyComparisonValue>
    where
        F: Fn(sequence::DynPyIter, sequence::DynPyIter) -> PyResult<bool>,
    {
        let r = if let Some(other) = other.payload_if_subclass::<PyList>(vm) {
            Implemented(op(
                self.borrow_elements().boxed_iter(),
                other.borrow_elements().boxed_iter(),
            )?)
        } else {
            NotImplemented
        };
        Ok(r)
    }

    #[pymethod(name = "__eq__")]
    fn eq(
        zelf: PyRef<Self>,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        if zelf.as_object().is(&other) {
            Ok(Implemented(true))
        } else {
            zelf.cmp(other, |a, b| sequence::eq(vm, a, b), vm)
        }
    }

    #[pymethod(name = "__ne__")]
    fn ne(
        zelf: PyRef<Self>,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        Ok(PyList::eq(zelf, other, vm)?.map(|v| !v))
    }

    #[pymethod(name = "__lt__")]
    fn lt(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyComparisonValue> {
        self.cmp(other, |a, b| sequence::lt(vm, a, b), vm)
    }

    #[pymethod(name = "__gt__")]
    fn gt(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyComparisonValue> {
        self.cmp(other, |a, b| sequence::gt(vm, a, b), vm)
    }

    #[pymethod(name = "__ge__")]
    fn ge(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyComparisonValue> {
        self.cmp(other, |a, b| sequence::ge(vm, a, b), vm)
    }

    #[pymethod(name = "__le__")]
    fn le(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyComparisonValue> {
        self.cmp(other, |a, b| sequence::le(vm, a, b), vm)
    }

    #[pymethod(name = "__delitem__")]
    fn delitem(&self, subscript: SequenceIndex, vm: &VirtualMachine) -> PyResult<()> {
        match subscript {
            SequenceIndex::Int(index) => self.delindex(index, vm),
            SequenceIndex::Slice(slice) => self.delslice(slice, vm),
        }
    }

    fn delindex(&self, index: isize, vm: &VirtualMachine) -> PyResult<()> {
        let mut elements = self.borrow_elements_mut();
        if let Some(pos_index) = get_pos(index, elements.len()) {
            elements.remove(pos_index);
            Ok(())
        } else {
            Err(vm.new_index_error("Index out of bounds!".to_owned()))
        }
    }

    fn delslice(&self, slice: PySliceRef, vm: &VirtualMachine) -> PyResult<()> {
        let start = slice.start_index(vm)?;
        let stop = slice.stop_index(vm)?;
        let step = slice.step_index(vm)?.unwrap_or_else(BigInt::one);

        let elements = self.borrow_elements_mut();
        if step.is_zero() {
            Err(vm.new_value_error("slice step cannot be zero".to_owned()))
        } else if step.is_positive() {
            let range = get_slice_range(&start, &stop, elements.len());
            if range.start < range.end {
                #[allow(clippy::range_plus_one)]
                match step.to_isize() {
                    Some(1) => {
                        PyList::_del_slice(elements, range);
                        Ok(())
                    }
                    Some(num) => {
                        PyList::_del_stepped_slice(elements, range, num as usize);
                        Ok(())
                    }
                    None => {
                        PyList::_del_slice(elements, range.start..range.start + 1);
                        Ok(())
                    }
                }
            } else {
                // no del to do
                Ok(())
            }
        } else {
            // calculate the range for the reverse slice, first the bounds needs to be made
            // exclusive around stop, the lower number
            let start = start.as_ref().map(|x| {
                if *x == (-1).to_bigint().unwrap() {
                    elements.len() + BigInt::one() //.to_bigint().unwrap()
                } else {
                    x + 1
                }
            });
            let stop = stop.as_ref().map(|x| {
                if *x == (-1).to_bigint().unwrap() {
                    elements.len().to_bigint().unwrap()
                } else {
                    x + 1
                }
            });
            let range = get_slice_range(&stop, &start, elements.len());
            if range.start < range.end {
                match (-step).to_isize() {
                    Some(1) => {
                        PyList::_del_slice(elements, range);
                        Ok(())
                    }
                    Some(num) => {
                        PyList::_del_stepped_slice_reverse(elements, range, num as usize);
                        Ok(())
                    }
                    None => {
                        PyList::_del_slice(elements, range.end - 1..range.end);
                        Ok(())
                    }
                }
            } else {
                // no del to do
                Ok(())
            }
        }
    }

    fn _del_slice(mut elements: PyRwLockWriteGuard<'_, Vec<PyObjectRef>>, range: Range<usize>) {
        elements.drain(range);
    }

    fn _del_stepped_slice(
        mut elements: PyRwLockWriteGuard<'_, Vec<PyObjectRef>>,
        range: Range<usize>,
        step: usize,
    ) {
        // no easy way to delete stepped indexes so here is what we'll do
        let mut deleted = 0;
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

    fn _del_stepped_slice_reverse(
        mut elements: PyRwLockWriteGuard<'_, Vec<PyObjectRef>>,
        range: Range<usize>,
        step: usize,
    ) {
        // no easy way to delete stepped indexes so here is what we'll do
        let mut deleted = 0;
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

    #[pymethod]
    fn sort(&self, options: SortOptions, vm: &VirtualMachine) -> PyResult<()> {
        // replace list contents with [] for duration of sort.
        // this prevents keyfunc from messing with the list and makes it easy to
        // check if it tries to append elements to it.
        let mut elements = std::mem::take(self.borrow_elements_mut().deref_mut());
        do_sort(vm, &mut elements, options.key, options.reverse)?;
        std::mem::swap(self.borrow_elements_mut().deref_mut(), &mut elements);

        if !elements.is_empty() {
            return Err(vm.new_value_error("list modified during sort".to_owned()));
        }

        Ok(())
    }

    #[pyslot]
    fn tp_new(
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
            Some(ref func) => vm.invoke(func, vec![x.clone()])?,
        });
    }

    quicksort(vm, &mut keys, values)?;

    if reverse {
        values.reverse();
    }

    Ok(())
}

#[pyclass]
#[derive(Debug)]
pub struct PyListIterator {
    pub position: AtomicCell<usize>,
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
        let list = self.list.borrow_elements();
        let pos = self.position.fetch_add(1);
        if let Some(obj) = list.get(pos) {
            Ok(obj.clone())
        } else {
            Err(objiter::new_stop_iteration(vm))
        }
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>) -> PyRef<Self> {
        zelf
    }

    #[pymethod(name = "__length_hint__")]
    fn length_hint(&self) -> usize {
        let list = self.list.borrow_elements();
        let pos = self.position.load();
        list.len().saturating_sub(pos)
    }
}

#[pyclass]
#[derive(Debug)]
pub struct PyListReverseIterator {
    pub position: AtomicCell<isize>,
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
        let list = self.list.borrow_elements();
        let pos = self.position.fetch_sub(1);
        if pos > 0 {
            if let Some(ret) = list.get(pos as usize - 1) {
                return Ok(ret.clone());
            }
        }
        Err(objiter::new_stop_iteration(vm))
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>) -> PyRef<Self> {
        zelf
    }

    #[pymethod(name = "__length_hint__")]
    fn length_hint(&self) -> usize {
        std::cmp::max(self.position.load(), 0) as usize
    }
}

pub fn init(context: &PyContext) {
    let list_type = &context.types.list_type;
    PyList::extend_class(context, list_type);

    PyListIterator::extend_class(context, &context.types.listiterator_type);
    PyListReverseIterator::extend_class(context, &context.types.listreverseiterator_type);
}
