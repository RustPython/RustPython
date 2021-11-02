use num_traits::ToPrimitive;
use std::ops::Range;

use crate::builtins::int::PyInt;
// export through slicable module, not slice.
pub use crate::builtins::slice::{saturate_index, SaturatedSlice};
use crate::builtins::slice::{PySlice, SaturatedSliceIterator};
use crate::utils::Either;
use crate::VirtualMachine;
use crate::{PyObjectRef, PyRef, PyResult, TypeProtocol};

pub trait PySliceableSequenceMut {
    type Item: Clone;
    // as CPython, length of range and items could be different, function must act like Vec::splice()
    fn do_set_range(&mut self, range: Range<usize>, items: &[Self::Item]);
    fn do_replace_indexes<I>(&mut self, indexes: I, items: &[Self::Item])
    where
        I: Iterator<Item = usize>;
    fn do_delete_range(&mut self, range: Range<usize>);
    fn do_delete_indexes<I>(&mut self, range: Range<usize>, indexes: I)
    where
        I: Iterator<Item = usize>;
    fn as_slice(&self) -> &[Self::Item];

    fn set_slice_items_no_resize(
        &mut self,
        vm: &VirtualMachine,
        slice: SaturatedSlice,
        items: &[Self::Item],
    ) -> PyResult<()> {
        let (range, step, slicelen) = slice.adjust_indices(self.as_slice().len());
        if slicelen != items.len() {
            Err(vm
                .new_buffer_error("Existing exports of data: object cannot be re-sized".to_owned()))
        } else if step == 1 {
            self.do_set_range(range, items);
            Ok(())
        } else {
            self.do_replace_indexes(
                SaturatedSliceIterator::from_adjust_indices(range, step, slicelen),
                items,
            );
            Ok(())
        }
    }

    fn set_slice_items(
        &mut self,
        vm: &VirtualMachine,
        slice: SaturatedSlice,
        items: &[Self::Item],
    ) -> PyResult<()> {
        let (range, step, slicelen) = slice.adjust_indices(self.as_slice().len());
        if step == 1 {
            self.do_set_range(range, items);
            Ok(())
        } else if slicelen == items.len() {
            self.do_replace_indexes(
                SaturatedSliceIterator::from_adjust_indices(range, step, slicelen),
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

    fn delete_slice(&mut self, _vm: &VirtualMachine, slice: SaturatedSlice) -> PyResult<()> {
        let (range, step, slicelen) = slice.adjust_indices(self.as_slice().len());
        if slicelen == 0 {
            Ok(())
        } else if step == 1 {
            self.do_delete_range(range);
            Ok(())
        } else {
            self.do_delete_indexes(
                range.clone(),
                SaturatedSliceIterator::from_adjust_indices(range, step, slicelen).positive_order(),
            );
            Ok(())
        }
    }
}

impl<T: Clone> PySliceableSequenceMut for Vec<T> {
    type Item = T;

    fn as_slice(&self) -> &[Self::Item] {
        self.as_slice()
    }

    fn do_set_range(&mut self, range: Range<usize>, items: &[Self::Item]) {
        self.splice(range, items.to_vec());
    }

    fn do_replace_indexes<I>(&mut self, indexes: I, items: &[Self::Item])
    where
        I: Iterator<Item = usize>,
    {
        for (i, item) in indexes.zip(items) {
            self[i] = item.clone();
        }
    }

    fn do_delete_range(&mut self, range: Range<usize>) {
        self.drain(range);
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

pub trait PySliceableSequence {
    type Item;
    type Sliced;

    fn do_get(&self, index: usize) -> Self::Item;
    fn do_slice(&self, range: Range<usize>) -> Self::Sliced;
    fn do_slice_reverse(&self, range: Range<usize>) -> Self::Sliced;
    fn do_stepped_slice(&self, range: Range<usize>, step: usize) -> Self::Sliced;
    fn do_stepped_slice_reverse(&self, range: Range<usize>, step: usize) -> Self::Sliced;
    fn empty() -> Self::Sliced;

    fn len(&self) -> usize;
    fn is_empty(&self) -> bool;

    fn wrap_index(&self, p: isize) -> Option<usize> {
        wrap_index(p, self.len())
    }

    fn saturate_index(&self, p: isize) -> usize {
        saturate_index(p, self.len())
    }

    fn get_slice_items(
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
        } else {
            if step.is_negative() {
                self.do_stepped_slice_reverse(range, step.unsigned_abs())
            } else {
                self.do_stepped_slice(range, step.unsigned_abs())
            }
        };
        Ok(sliced)
    }

    fn get_item(
        &self,
        vm: &VirtualMachine,
        needle: PyObjectRef,
        owner_type: &'static str,
    ) -> PyResult<Either<Self::Item, Self::Sliced>> {
        let needle = SequenceIndex::try_from_object_for(vm, needle, owner_type)?;
        match needle {
            SequenceIndex::Int(value) => {
                let pos_index = self.wrap_index(value).ok_or_else(|| {
                    vm.new_index_error(format!("{} index out of range", owner_type))
                })?;
                Ok(Either::A(self.do_get(pos_index)))
            }
            SequenceIndex::Slice(slice) => {
                let slice = slice.to_saturated(vm)?;
                Ok(Either::B(self.get_slice_items(vm, slice)?))
            }
        }
    }
}

impl<T: Clone> PySliceableSequence for [T] {
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

    #[inline(always)]
    fn is_empty(&self) -> bool {
        self.is_empty()
    }
}

pub enum SequenceIndex {
    Int(isize),
    Slice(PyRef<PySlice>),
}

impl SequenceIndex {
    pub fn try_from_object_for(
        vm: &VirtualMachine,
        obj: PyObjectRef,
        owner_type: &'static str,
    ) -> PyResult<Self> {
        let idx = match_class!(match obj {
            i @ PyInt => i.as_bigint().to_isize(),
            s @ PySlice => return Ok(SequenceIndex::Slice(s)),
            obj => {
                let val = vm.to_index(&obj).map_err(|_| vm.new_type_error(format!(
                    "{} indices must be integers or slices or classes that override __index__ operator, not '{}'",
                    owner_type,
                    obj.class().name()
                )))?;
                val.as_bigint().to_isize()
            }
        }).ok_or_else(|| {
            vm.new_index_error("cannot fit 'int' into an index-sized integer".to_owned())
        })?;
        Ok(SequenceIndex::Int(idx))
    }
}

/// Get the index into a sequence like type. Get it from a python integer
/// object, accounting for negative index, and out of bounds issues.
// pub fn get_sequence_index(vm: &VirtualMachine, index: &PyIntRef, length: usize) -> PyResult<usize> {
//     if let Some(value) = index.as_bigint().to_i64() {
//         if value < 0 {
//             let from_end: usize = -value as usize;
//             if from_end > length {
//                 Err(vm.new_index_error("Index out of bounds!".to_owned()))
//             } else {
//                 let index = length - from_end;
//                 Ok(index)
//             }
//         } else {
//             let index = value as usize;
//             if index >= length {
//                 Err(vm.new_index_error("Index out of bounds!".to_owned()))
//             } else {
//                 Ok(index)
//             }
//         }
//     } else {
//         Err(vm.new_index_error("cannot fit 'int' into an index-sized integer".to_owned()))
//     }
// }

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
