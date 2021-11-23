use num_traits::ToPrimitive;
use std::ops::Range;

// export through slicable module, not slice.
pub use crate::builtins::slice::{saturate_index, SaturatedSlice};
use crate::{
    builtins::{
        int::PyInt,
        slice::{PySlice, SaturatedSliceIter},
    },
    utils::Either,
    PyObject, PyResult, TypeProtocol, VirtualMachine,
};

pub trait SliceableSequenceMutOp {
    type Item: Clone;
    fn as_slice(&self) -> &[Self::Item];
    fn do_set(&mut self, index: usize, value: Self::Item);
    fn do_delele(&mut self, index: usize);
    /// as CPython, length of range and items could be different, function must act like Vec::splice()
    fn do_set_range(&mut self, range: Range<usize>, items: &[Self::Item]);
    fn do_delete_range(&mut self, range: Range<usize>);
    fn do_set_indexes<I>(&mut self, indexes: I, items: &[Self::Item])
    where
        I: Iterator<Item = usize>;
    /// indexes must be positive order
    fn do_delete_indexes<I>(&mut self, range: Range<usize>, indexes: I)
    where
        I: Iterator<Item = usize>;

    fn set_item_by_index(
        &mut self,
        vm: &VirtualMachine,
        index: isize,
        value: Self::Item,
    ) -> PyResult<()> {
        let pos = self
            .as_slice()
            .wrap_index(index)
            .ok_or_else(|| vm.new_index_error("assigment index out of range".to_owned()))?;
        self.do_set(pos, value);
        Ok(())
    }

    fn set_item_by_slice_no_resize(
        &mut self,
        vm: &VirtualMachine,
        slice: &PySlice,
        items: &[Self::Item],
    ) -> PyResult<()> {
        let slice = slice.to_saturated(vm)?;
        let (range, step, slicelen) = slice.adjust_indices(self.as_slice().len());
        if slicelen != items.len() {
            Err(vm
                .new_buffer_error("Existing exports of data: object cannot be re-sized".to_owned()))
        } else if step == 1 {
            self.do_set_range(range, items);
            Ok(())
        } else {
            self.do_set_indexes(
                SaturatedSliceIter::from_adjust_indices(range, step, slicelen),
                items,
            );
            Ok(())
        }
    }

    fn set_item_by_slice(
        &mut self,
        vm: &VirtualMachine,
        slice: &PySlice,
        items: &[Self::Item],
    ) -> PyResult<()> {
        let slice = slice.to_saturated(vm)?;
        let (range, step, slicelen) = slice.adjust_indices(self.as_slice().len());
        if step == 1 {
            self.do_set_range(range, items);
            Ok(())
        } else if slicelen == items.len() {
            self.do_set_indexes(
                SaturatedSliceIter::from_adjust_indices(range, step, slicelen),
                items,
            );
            Ok(())
        } else {
            Err(vm.new_value_error(format!(
                "attempt to assign sequence of size {} to extended slice of size {}",
                items.len(),
                slicelen
            )))
        }
    }

    fn del_item_by_index(&mut self, vm: &VirtualMachine, index: isize) -> PyResult<()> {
        let pos = self
            .as_slice()
            .wrap_index(index)
            .ok_or_else(|| vm.new_index_error("assigment index out of range".to_owned()))?;
        self.do_delele(pos);
        Ok(())
    }

    fn del_item_by_slice(&mut self, vm: &VirtualMachine, slice: &PySlice) -> PyResult<()> {
        let slice = slice.to_saturated(vm)?;
        let (range, step, slicelen) = slice.adjust_indices(self.as_slice().len());
        if slicelen == 0 {
            Ok(())
        } else if step == 1 {
            self.do_set_range(range, &[]);
            Ok(())
        } else {
            self.do_delete_indexes(
                range.clone(),
                SaturatedSliceIter::from_adjust_indices(range, step, slicelen).positive_order(),
            );
            Ok(())
        }
    }

    fn del_item(&mut self, vm: &VirtualMachine, needle: &PyObject) -> PyResult<()> {
        let needle = SequenceIndex::try_borrow_from_object(vm, needle)?;
        match needle {
            SequenceIndex::Int(index) => self.del_item_by_index(vm, index),
            SequenceIndex::Slice(slice) => self.del_item_by_slice(vm, slice),
        }
    }
}

impl<T: Clone> SliceableSequenceMutOp for Vec<T> {
    type Item = T;

    fn as_slice(&self) -> &[Self::Item] {
        self.as_slice()
    }

    fn do_set(&mut self, index: usize, value: Self::Item) {
        self[index] = value;
    }

    fn do_delele(&mut self, index: usize) {
        self.remove(index);
    }

    fn do_set_range(&mut self, range: Range<usize>, items: &[Self::Item]) {
        self.splice(range, items.to_vec());
    }

    fn do_delete_range(&mut self, range: Range<usize>) {
        self.drain(range);
    }

    fn do_set_indexes<I>(&mut self, indexes: I, items: &[Self::Item])
    where
        I: Iterator<Item = usize>,
    {
        for (i, item) in indexes.zip(items) {
            self.do_set(i, item.clone());
        }
    }

    fn do_delete_indexes<I>(&mut self, range: Range<usize>, indexes: I)
    where
        I: Iterator<Item = usize>,
    {
        let mut indexes = indexes.peekable();
        let mut deleted = 0;

        // passing whole range, swap or overlap
        for i in range.clone() {
            if indexes.peek() == Some(&i) {
                indexes.next();
                deleted += 1;
            } else {
                self.swap(i - deleted, i);
            }
        }
        // then drain (the values to delete should now be contiguous at the end of the range)
        self.drain((range.end - deleted)..range.end);
    }
}

#[allow(clippy::len_without_is_empty)]
pub trait SliceableSequenceOp {
    type Item;
    type Sliced;

    fn do_get(&self, index: usize) -> Self::Item;
    fn do_slice(&self, range: Range<usize>) -> Self::Sliced;
    fn do_slice_reverse(&self, range: Range<usize>) -> Self::Sliced;
    fn do_stepped_slice(&self, range: Range<usize>, step: usize) -> Self::Sliced;
    fn do_stepped_slice_reverse(&self, range: Range<usize>, step: usize) -> Self::Sliced;
    fn empty() -> Self::Sliced;

    fn len(&self) -> usize;

    fn wrap_index(&self, p: isize) -> Option<usize> {
        wrap_index(p, self.len())
    }

    fn saturate_index(&self, p: isize) -> usize {
        saturate_index(p, self.len())
    }

    fn get_item_by_slice(&self, vm: &VirtualMachine, slice: &PySlice) -> PyResult<Self::Sliced> {
        let slice = slice.to_saturated(vm)?;
        let (range, step, slicelen) = slice.adjust_indices(self.len());
        let sliced = if slicelen == 0 {
            Self::empty()
        } else if step == 1 {
            if step.is_negative() {
                self.do_slice_reverse(range)
            } else {
                self.do_slice(range)
            }
        } else if step.is_negative() {
            self.do_stepped_slice_reverse(range, step.unsigned_abs())
        } else {
            self.do_stepped_slice(range, step.unsigned_abs())
        };
        Ok(sliced)
    }

    fn get_item_by_index(&self, vm: &VirtualMachine, index: isize) -> PyResult<Self::Item> {
        let pos = self
            .wrap_index(index)
            .ok_or_else(|| vm.new_index_error("index out of range".to_owned()))?;
        Ok(self.do_get(pos))
    }

    fn get_item(
        &self,
        vm: &VirtualMachine,
        needle: &PyObject,
    ) -> PyResult<Either<Self::Item, Self::Sliced>> {
        let needle = SequenceIndex::try_borrow_from_object(vm, needle)?;
        match needle {
            SequenceIndex::Int(index) => self.get_item_by_index(vm, index).map(Either::A),
            SequenceIndex::Slice(slice) => self.get_item_by_slice(vm, slice).map(Either::B),
        }
    }
}

impl<T: Clone> SliceableSequenceOp for [T] {
    type Item = T;
    type Sliced = Vec<T>;

    #[inline]
    fn do_get(&self, index: usize) -> Self::Item {
        self[index].clone()
    }

    #[inline]
    fn do_slice(&self, range: Range<usize>) -> Self::Sliced {
        self[range].to_vec()
    }

    #[inline]
    fn do_slice_reverse(&self, range: Range<usize>) -> Self::Sliced {
        let mut slice = self[range].to_vec();
        slice.reverse();
        slice
    }

    #[inline]
    fn do_stepped_slice(&self, range: Range<usize>, step: usize) -> Self::Sliced {
        self[range].iter().step_by(step).cloned().collect()
    }

    #[inline]
    fn do_stepped_slice_reverse(&self, range: Range<usize>, step: usize) -> Self::Sliced {
        self[range].iter().rev().step_by(step).cloned().collect()
    }

    #[inline(always)]
    fn empty() -> Self::Sliced {
        Vec::new()
    }

    #[inline(always)]
    fn len(&self) -> usize {
        self.len()
    }
}

pub enum SequenceIndex<'a> {
    Int(isize),
    Slice(&'a PySlice),
}

impl<'a> SequenceIndex<'a> {
    pub fn try_borrow_from_object(vm: &VirtualMachine, obj: &'a PyObject) -> PyResult<Self> {
        if let Some(index) = obj.payload::<PyInt>() {
            // TODO: replace by number protocol
            index
                .as_bigint()
                .to_isize()
                .ok_or_else(|| {
                    vm.new_index_error("cannot fit 'int' into an index-sized integer".to_owned())
                })
                .map(Self::Int)
        } else if let Some(slice) = obj.payload::<PySlice>() {
            Ok(Self::Slice(slice))
        } else if let Some(index) = vm.to_index_opt(obj.to_owned()) {
            // TODO: __index__ for indice is no more supported
            index?
                .as_bigint()
                .to_isize()
                .ok_or_else(|| {
                    vm.new_index_error("cannot fit 'int' into an index-sized integer".to_owned())
                })
                .map(Self::Int)
        } else {
            Err(vm.new_type_error(format!(
                "indices must be integers or slices or classes that override __index__ operator, not '{}'",
                obj.class().name()
            )))
        }
    }
}

// Use PySliceableSequence::wrap_index for implementors
pub(crate) fn wrap_index(p: isize, len: usize) -> Option<usize> {
    let neg = p.is_negative();
    let p = p.wrapping_abs() as usize;
    if neg {
        len.checked_sub(p)
    } else if p >= len {
        None
    } else {
        Some(p)
    }
}
