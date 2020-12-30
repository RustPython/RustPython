use num_bigint::BigInt;
use num_traits::{One, Signed, ToPrimitive, Zero};
use std::ops::Range;

use crate::builtins::int::PyInt;
use crate::builtins::slice::{PySlice, PySliceRef};
use crate::pyobject::{BorrowValue, Either, PyObjectRef, PyResult, TryFromObject, TypeProtocol};
use crate::vm::VirtualMachine;

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
        slice: &PySlice,
        items: &[Self::Item],
    ) -> PyResult<()> {
        let (range, step, is_negative_step) = convert_slice(slice, self.as_slice().len(), vm)?;
        if !is_negative_step && step == Some(1) {
            return if range.end - range.start == items.len() {
                self.do_set_range(range, items);
                Ok(())
            } else {
                Err(vm.new_buffer_error(
                    "Existing exports of data: object cannot be re-sized".to_owned(),
                ))
            };
        }
        if let Some(step) = step {
            let slicelen = if range.end > range.start {
                (range.end - range.start - 1) / step + 1
            } else {
                0
            };

            if slicelen == items.len() {
                let indexes = if is_negative_step {
                    itertools::Either::Left(range.rev().step_by(step))
                } else {
                    itertools::Either::Right(range.step_by(step))
                };
                self.do_replace_indexes(indexes, items);
                Ok(())
            } else {
                Err(vm.new_buffer_error(
                    "Existing exports of data: object cannot be re-sized".to_owned(),
                ))
            }
        } else {
            // edge case, step is too big for usize
            // same behaviour as CPython
            let slicelen = if range.start < range.end { 1 } else { 0 };
            if match items.len() {
                0 => slicelen == 0,
                1 => {
                    self.do_set_range(range, items);
                    true
                }
                _ => false,
            } {
                Ok(())
            } else {
                Err(vm.new_buffer_error(
                    "Existing exports of data: object cannot be re-sized".to_owned(),
                ))
            }
        }
    }

    fn set_slice_items(
        &mut self,
        vm: &VirtualMachine,
        slice: &PySlice,
        items: &[Self::Item],
    ) -> PyResult<()> {
        let (range, step, is_negative_step) = convert_slice(slice, self.as_slice().len(), vm)?;
        if !is_negative_step && step == Some(1) {
            self.do_set_range(range, items);
            return Ok(());
        }
        if let Some(step) = step {
            let slicelen = if range.end > range.start {
                (range.end - range.start - 1) / step + 1
            } else {
                0
            };

            if slicelen == items.len() {
                let indexes = if is_negative_step {
                    itertools::Either::Left(range.rev().step_by(step))
                } else {
                    itertools::Either::Right(range.step_by(step))
                };
                self.do_replace_indexes(indexes, items);
                Ok(())
            } else {
                Err(vm.new_value_error(format!(
                    "attempt to assign sequence of size {} to extended slice of size {}",
                    items.len(),
                    slicelen
                )))
            }
        } else {
            // edge case, step is too big for usize
            // same behaviour as CPython
            let slicelen = if range.start < range.end { 1 } else { 0 };
            if match items.len() {
                0 => slicelen == 0,
                1 => {
                    self.do_set_range(range, items);
                    true
                }
                _ => false,
            } {
                Ok(())
            } else {
                Err(vm.new_value_error(format!(
                    "attempt to assign sequence of size {} to extended slice of size {}",
                    items.len(),
                    slicelen
                )))
            }
        }
    }

    fn delete_slice(&mut self, vm: &VirtualMachine, slice: &PySlice) -> PyResult<()> {
        let (range, step, is_negative_step) = convert_slice(slice, self.as_slice().len(), vm)?;
        if range.start >= range.end {
            return Ok(());
        }

        if !is_negative_step && step == Some(1) {
            self.do_delete_range(range);
            return Ok(());
        }

        // step is not negative here
        if let Some(step) = step {
            let indexes = if is_negative_step {
                itertools::Either::Left(range.clone().rev().step_by(step).rev())
            } else {
                itertools::Either::Right(range.clone().step_by(step))
            };

            self.do_delete_indexes(range, indexes);
        } else {
            // edge case, step is too big for usize
            // same behaviour as CPython
            self.do_delete_range(range);
        }
        Ok(())
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

    fn saturate_big_index(&self, slice_pos: &BigInt) -> usize {
        saturate_big_index(slice_pos, self.len())
    }

    fn saturate_range(&self, start: &Option<BigInt>, stop: &Option<BigInt>) -> Range<usize> {
        saturate_range(start, stop, self.len())
    }

    fn get_slice_items(&self, vm: &VirtualMachine, slice: &PySlice) -> PyResult<Self::Sliced> {
        let (range, step, is_negative_step) = convert_slice(slice, self.len(), vm)?;
        if range.start >= range.end {
            return Ok(Self::empty());
        }

        if step == Some(1) {
            return Ok(if is_negative_step {
                self.do_slice_reverse(range)
            } else {
                self.do_slice(range)
            });
        }

        if let Some(step) = step {
            Ok(if is_negative_step {
                self.do_stepped_slice_reverse(range, step)
            } else {
                self.do_stepped_slice(range, step)
            })
        } else {
            Ok(self.do_slice(range))
        }
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
            SequenceIndex::Slice(slice) => Ok(Either::B(self.get_slice_items(vm, &slice)?)),
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
    Slice(PySliceRef),
}

impl SequenceIndex {
    fn try_from_object_for(
        vm: &VirtualMachine,
        obj: PyObjectRef,
        owner_type: &'static str,
    ) -> PyResult<Self> {
        match_class!(match obj {
            i @ PyInt => i
                .borrow_value()
                .to_isize()
                .map(SequenceIndex::Int)
                .ok_or_else(|| vm
                    .new_index_error("cannot fit 'int' into an index-sized integer".to_owned())),
            s @ PySlice => Ok(SequenceIndex::Slice(s)),
            obj => Err(vm.new_type_error(format!(
                "{} indices must be integers or slices, not {}",
                owner_type,
                obj.class().name,
            ))),
        })
    }
}

impl TryFromObject for SequenceIndex {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        Self::try_from_object_for(vm, obj, "sequence")
    }
}

/// Get the index into a sequence like type. Get it from a python integer
/// object, accounting for negative index, and out of bounds issues.
// pub fn get_sequence_index(vm: &VirtualMachine, index: &PyIntRef, length: usize) -> PyResult<usize> {
//     if let Some(value) = index.borrow_value().to_i64() {
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

// return pos is in range [0, len] inclusive
pub(crate) fn saturate_index(p: isize, len: usize) -> usize {
    let mut p = p;
    let len = len.to_isize().unwrap();
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

fn saturate_big_index(slice_pos: &BigInt, len: usize) -> usize {
    if let Some(pos) = slice_pos.to_isize() {
        saturate_index(pos, len)
    } else if slice_pos.is_negative() {
        // slice past start bound, round to start
        0
    } else {
        // slice past end bound, round to end
        len
    }
}

pub(crate) fn saturate_range(
    start: &Option<BigInt>,
    stop: &Option<BigInt>,
    len: usize,
) -> Range<usize> {
    let start = start.as_ref().map_or(0, |x| saturate_big_index(x, len));
    let stop = stop.as_ref().map_or(len, |x| saturate_big_index(x, len));

    start..stop
}

pub(crate) fn convert_slice(
    slice: &PySlice,
    len: usize,
    vm: &VirtualMachine,
) -> PyResult<(Range<usize>, Option<usize>, bool)> {
    let start = slice.start_index(vm)?;
    let stop = slice.stop_index(vm)?;
    let step = slice.step_index(vm)?.unwrap_or_else(BigInt::one);

    if step.is_zero() {
        return Err(vm.new_value_error("slice step cannot be zero".to_owned()));
    }

    let (start, stop, step, is_negative_step) = if step.is_negative() {
        (
            stop.map(|x| {
                if x == -BigInt::one() {
                    len + BigInt::one()
                } else {
                    x + 1
                }
            }),
            start.map(|x| {
                if x == -BigInt::one() {
                    BigInt::from(len)
                } else {
                    x + 1
                }
            }),
            -step,
            true,
        )
    } else {
        (start, stop, step, false)
    };

    let step = step.to_usize();

    let range = saturate_range(&start, &stop, len);
    let range = if range.start >= range.end {
        range.start..range.start
    } else {
        // step overflow
        if step.is_none() {
            if is_negative_step {
                (range.end - 1)..range.end
            } else {
                range.start..(range.start + 1)
            }
        } else {
            range
        }
    };

    Ok((range, step, is_negative_step))
}
