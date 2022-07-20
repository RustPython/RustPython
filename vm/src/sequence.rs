use crate::{
    builtins::PyIntRef, function::OptionalArg, sliceable::SequenceIndexOp, types::PyComparisonOp,
    vm::VirtualMachine, AsObject, PyObject, PyObjectRef, PyResult,
};
use optional::Optioned;
use std::ops::Range;

pub trait MutObjectSequenceOp<'a>: Sized {
    type Guard: Sized;

    fn do_get(index: usize, guard: &Self::Guard) -> Option<&PyObjectRef>;
    fn do_lock(&'a self) -> Self::Guard;

    fn mut_count(&'a self, vm: &VirtualMachine, needle: &PyObject) -> PyResult<usize> {
        let mut find_iter = self._mut_find(vm, needle, 0..isize::MAX as usize);
        let mut count = 0;
        while (find_iter.next().transpose()?).is_some() {
            count += 1;
        }
        Ok(count)
    }

    fn mut_index_range(
        &'a self,
        vm: &VirtualMachine,
        needle: &PyObject,
        range: Range<usize>,
    ) -> PyResult<Option<usize>> {
        self._mut_find(vm, needle, range).next().transpose()
    }

    fn mut_index(&'a self, vm: &VirtualMachine, needle: &PyObject) -> PyResult<Option<usize>> {
        self.mut_index_range(vm, needle, 0..isize::MAX as usize)
    }

    fn mut_contains(&'a self, vm: &VirtualMachine, needle: &PyObject) -> PyResult<bool> {
        self.mut_index(vm, needle).map(|x| x.is_some())
    }

    fn _mut_find<'b, 'vm>(
        &'a self,
        vm: &'vm VirtualMachine,
        needle: &'b PyObject,
        range: Range<usize>,
    ) -> MutObjectSequenceFindIter<'a, 'b, 'vm, Self> {
        MutObjectSequenceFindIter {
            seq: self,
            needle,
            pos: range.start,
            end: range.end,
            guard: None,
            vm,
        }
    }
}

pub struct MutObjectSequenceFindIter<'a, 'b, 'vm, S: MutObjectSequenceOp<'a>> {
    // mutable fields
    pos: usize,
    guard: Option<S::Guard>,
    // immutable fields
    seq: &'a S,
    needle: &'b PyObject,
    end: usize,
    vm: &'vm VirtualMachine,
}

impl<'a, 'b, 'vm, S: MutObjectSequenceOp<'a>> MutObjectSequenceFindIter<'a, 'b, 'vm, S> {
    #[inline]
    fn next_impl(&mut self) -> PyResult<Optioned<usize>> {
        loop {
            if self.pos >= self.end {
                return Ok(Optioned::none());
            }
            let guard = self.guard.take().unwrap_or_else(|| self.seq.do_lock());
            let elem = if let Some(x) = S::do_get(self.pos, &guard) {
                x
            } else {
                return Ok(Optioned::none());
            };

            let is_equal = if elem.is(self.needle) {
                self.guard = Some(guard);
                true
            } else {
                let elem = elem.clone();
                drop(guard);
                elem.rich_compare_bool(self.needle, PyComparisonOp::Eq, self.vm)?
            };

            if is_equal {
                break;
            }

            self.pos += 1;
        }

        let i = self.pos;
        self.pos += 1;
        Ok(Optioned::some(i))
    }
}

impl<'a, 'b, 'vm, S: MutObjectSequenceOp<'a>> Iterator
    for MutObjectSequenceFindIter<'a, 'b, 'vm, S>
{
    type Item = PyResult<usize>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.next_impl().map(Into::into).transpose()
    }
}

pub trait SequenceExt<T: Clone>
where
    Self: AsRef<[T]>,
{
    fn mul(&self, vm: &VirtualMachine, n: isize) -> PyResult<Vec<T>> {
        let n = vm.check_repeat_or_overflow_error(self.as_ref().len(), n)?;
        let mut v = Vec::with_capacity(n * self.as_ref().len());
        for _ in 0..n {
            v.extend_from_slice(self.as_ref());
        }
        Ok(v)
    }
}

impl<T: Clone> SequenceExt<T> for [T] {}

pub trait SequenceMutExt<T: Clone>
where
    Self: AsRef<[T]>,
{
    fn as_vec_mut(&mut self) -> &mut Vec<T>;

    fn imul(&mut self, vm: &VirtualMachine, n: isize) -> PyResult<()> {
        let n = vm.check_repeat_or_overflow_error(self.as_ref().len(), n)?;
        if n == 0 {
            self.as_vec_mut().clear();
        } else if n != 1 {
            let mut sample = self.as_vec_mut().clone();
            if n != 2 {
                self.as_vec_mut().reserve(sample.len() * (n - 1));
                for _ in 0..n - 2 {
                    self.as_vec_mut().extend_from_slice(&sample);
                }
            }
            self.as_vec_mut().append(&mut sample);
        }
        Ok(())
    }
}

impl<T: Clone> SequenceMutExt<T> for Vec<T> {
    fn as_vec_mut(&mut self) -> &mut Vec<T> {
        self
    }
}

#[derive(FromArgs)]
pub struct OptionalRangeArgs {
    #[pyarg(positional, optional)]
    start: OptionalArg<PyObjectRef>,
    #[pyarg(positional, optional)]
    stop: OptionalArg<PyObjectRef>,
}

impl OptionalRangeArgs {
    pub fn saturate(self, len: usize, vm: &VirtualMachine) -> PyResult<(usize, usize)> {
        let saturate = |obj: PyObjectRef| -> PyResult<_> {
            obj.try_into_value(vm)
                .map(|int: PyIntRef| int.as_bigint().saturated_at(len))
        };
        let start = self.start.map_or(Ok(0), saturate)?;
        let stop = self.stop.map_or(Ok(len), saturate)?;
        Ok((start, stop))
    }
}
