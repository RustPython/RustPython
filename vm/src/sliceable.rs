// export through slicable module, not slice.
use crate::{
    builtins::{int::PyInt, slice::PySlice},
    AsObject, PyObject, PyResult, VirtualMachine,
};
use num_traits::{Signed, ToPrimitive};
use std::ops::Range;

pub trait SliceableSequenceMutOp
where
    Self: AsRef<[Self::Item]>,
{
    type Item: Clone;
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
            .as_ref()
            .wrap_index(index)
            .ok_or_else(|| vm.new_index_error("assigment index out of range".to_owned()))?;
        self.do_set(pos, value);
        Ok(())
    }

    fn set_item_by_slice_no_resize(
        &mut self,
        vm: &VirtualMachine,
        slice: SaturatedSlice,
        items: &[Self::Item],
    ) -> PyResult<()> {
        let (range, step, slicelen) = slice.adjust_indices(self.as_ref().len());
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
        slice: SaturatedSlice,
        items: &[Self::Item],
    ) -> PyResult<()> {
        let (range, step, slicelen) = slice.adjust_indices(self.as_ref().len());
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
            .as_ref()
            .wrap_index(index)
            .ok_or_else(|| vm.new_index_error("assigment index out of range".to_owned()))?;
        self.do_delele(pos);
        Ok(())
    }

    fn del_item_by_slice(&mut self, _vm: &VirtualMachine, slice: SaturatedSlice) -> PyResult<()> {
        let (range, step, slicelen) = slice.adjust_indices(self.as_ref().len());
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
}

impl<T: Clone> SliceableSequenceMutOp for Vec<T> {
    type Item = T;

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

    fn get_item_by_slice(
        &self,
        _vm: &VirtualMachine,
        slice: SaturatedSlice,
    ) -> PyResult<Self::Sliced> {
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

pub enum SequenceIndex {
    Int(isize),
    Slice(SaturatedSlice),
}

impl SequenceIndex {
    pub fn try_from_borrowed_object(vm: &VirtualMachine, obj: &PyObject, type_name: &str) -> PyResult<Self> {
        if let Some(i) = obj.payload::<PyInt>() {
            // TODO: number protocol
            i.try_to_primitive(vm)
                .map_err(|_| {
                    vm.new_index_error("cannot fit 'int' into an index-sized integer".to_owned())
                })
                .map(Self::Int)
        } else if let Some(slice) = obj.payload::<PySlice>() {
            slice.to_saturated(vm).map(Self::Slice)
        } else if let Some(i) = vm.to_index_opt(obj.to_owned()) {
            // TODO: __index__ for indice is no more supported?
            i?.try_to_primitive(vm)
                .map_err(|_| {
                    vm.new_index_error("cannot fit 'int' into an index-sized integer".to_owned())
                })
                .map(Self::Int)
        } else {
            Err(vm.new_type_error(format!(
                "{} indices must be integers or slices or classes that override __index__ operator, not '{}'",
                type_name,
                obj.class()
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

// Saturate p in range [0, len] inclusive
pub fn saturate_index(p: isize, len: usize) -> usize {
    let len = len.to_isize().unwrap_or(isize::MAX);
    let mut p = p;
    if p < 0 {
        p += len;
        if p < 0 {
            p = 0;
        }
    }
    if p > len {
        p = len;
    }
    p as usize
}

/// A saturated slice with values ranging in [isize::MIN, isize::MAX]. Used for
/// slicable sequences that require indices in the aforementioned range.
///
/// Invokes `__index__` on the PySliceRef during construction so as to separate the
/// transformation from PyObject into isize and the adjusting of the slice to a given
/// sequence length. The reason this is important is due to the fact that an objects
/// `__index__` might get a lock on the sequence and cause a deadlock.
#[derive(Copy, Clone, Debug)]
pub struct SaturatedSlice {
    start: isize,
    stop: isize,
    step: isize,
}

impl SaturatedSlice {
    // Equivalent to PySlice_Unpack.
    pub fn with_slice(slice: &PySlice, vm: &VirtualMachine) -> PyResult<Self> {
        let step = to_isize_index(vm, slice.step_ref(vm))?.unwrap_or(1);
        if step == 0 {
            return Err(vm.new_value_error("slice step cannot be zero".to_owned()));
        }
        let start = to_isize_index(vm, slice.start_ref(vm))?.unwrap_or_else(|| {
            if step.is_negative() {
                isize::MAX
            } else {
                0
            }
        });

        let stop = to_isize_index(vm, &slice.stop(vm))?.unwrap_or_else(|| {
            if step.is_negative() {
                isize::MIN
            } else {
                isize::MAX
            }
        });
        Ok(Self { start, stop, step })
    }

    // Equivalent to PySlice_AdjustIndices
    /// Convert for usage in indexing the underlying rust collections. Called *after*
    /// __index__ has been called on the Slice which might mutate the collection.
    pub fn adjust_indices(&self, len: usize) -> (Range<usize>, isize, usize) {
        if len == 0 {
            return (0..0, self.step, 0);
        }
        let range = if self.step.is_negative() {
            let stop = if self.stop == -1 {
                len
            } else {
                saturate_index(self.stop.saturating_add(1), len)
            };
            let start = if self.start == -1 {
                len
            } else {
                saturate_index(self.start.saturating_add(1), len)
            };
            stop..start
        } else {
            saturate_index(self.start, len)..saturate_index(self.stop, len)
        };

        let (range, slicelen) = if range.start >= range.end {
            (range.start..range.start, 0)
        } else {
            let slicelen = (range.end - range.start - 1) / self.step.unsigned_abs() + 1;
            (range, slicelen)
        };
        (range, self.step, slicelen)
    }

    pub fn iter(&self, len: usize) -> SaturatedSliceIter {
        SaturatedSliceIter::new(self, len)
    }
}

pub struct SaturatedSliceIter {
    index: isize,
    step: isize,
    len: usize,
}

impl SaturatedSliceIter {
    pub fn new(slice: &SaturatedSlice, seq_len: usize) -> Self {
        let (range, step, len) = slice.adjust_indices(seq_len);
        Self::from_adjust_indices(range, step, len)
    }

    pub fn from_adjust_indices(range: Range<usize>, step: isize, len: usize) -> Self {
        let index = if step.is_negative() {
            range.end as isize - 1
        } else {
            range.start as isize
        };
        Self { index, step, len }
    }

    pub fn positive_order(mut self) -> Self {
        if self.step.is_negative() {
            self.index += self.step * self.len.saturating_sub(1) as isize;
            self.step = self.step.saturating_abs()
        }
        self
    }
}

impl Iterator for SaturatedSliceIter {
    type Item = usize;

    fn next(&mut self) -> Option<Self::Item> {
        if self.len == 0 {
            return None;
        }
        self.len -= 1;
        let ret = self.index as usize;
        // SAFETY: if index is overflowed, len should be zero
        self.index = self.index.wrapping_add(self.step);
        Some(ret)
    }
}

// Go from PyObjectRef to isize w/o overflow error, out of range values are substituted by
// isize::MIN or isize::MAX depending on type and value of step.
// Equivalent to PyEval_SliceIndex.
fn to_isize_index(vm: &VirtualMachine, obj: &PyObject) -> PyResult<Option<isize>> {
    if vm.is_none(obj) {
        return Ok(None);
    }
    let result = vm.to_index_opt(obj.to_owned()).unwrap_or_else(|| {
        Err(vm.new_type_error(
            "slice indices must be integers or None or have an __index__ method".to_owned(),
        ))
    })?;
    let value = result.as_bigint();
    let is_negative = value.is_negative();
    Ok(Some(value.to_isize().unwrap_or(if is_negative {
        isize::MIN
    } else {
        isize::MAX
    })))
}
