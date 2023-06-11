use crate::{
    builtins::PyIntRef, function::OptionalArg, sliceable::SequenceIndexOp, types::PyComparisonOp,
    vm::VirtualMachine, AsObject, PyObject, PyObjectRef, PyResult,
};
use optional::Optioned;
use std::ops::Range;

pub trait MutObjectSequenceOp {
    type Guard<'a>: 'a;

    fn do_get<'a>(index: usize, guard: &'a Self::Guard<'_>) -> Option<&'a PyObjectRef>;
    fn do_lock(&self) -> Self::Guard<'_>;

    fn mut_count(&self, vm: &VirtualMachine, needle: &PyObject) -> PyResult<usize> {
        let mut count = 0;
        self._mut_iter_equal_skeleton::<_, false>(vm, needle, 0..isize::MAX as usize, || {
            count += 1
        })?;
        Ok(count)
    }

    fn mut_index_range(
        &self,
        vm: &VirtualMachine,
        needle: &PyObject,
        range: Range<usize>,
    ) -> PyResult<Optioned<usize>> {
        self._mut_iter_equal_skeleton::<_, true>(vm, needle, range, || {})
    }

    fn mut_index(&self, vm: &VirtualMachine, needle: &PyObject) -> PyResult<Optioned<usize>> {
        self.mut_index_range(vm, needle, 0..isize::MAX as usize)
    }

    fn mut_contains(&self, vm: &VirtualMachine, needle: &PyObject) -> PyResult<bool> {
        self.mut_index(vm, needle).map(|x| x.is_some())
    }

    fn _mut_iter_equal_skeleton<F, const SHORT: bool>(
        &self,
        vm: &VirtualMachine,
        needle: &PyObject,
        range: Range<usize>,
        mut f: F,
    ) -> PyResult<Optioned<usize>>
    where
        F: FnMut(),
    {
        let mut borrower = None;
        let mut i = range.start;

        let index = loop {
            if i >= range.end {
                break Optioned::<usize>::none();
            }
            let guard = if let Some(x) = borrower.take() {
                x
            } else {
                self.do_lock()
            };

            let elem = if let Some(x) = Self::do_get(i, &guard) {
                x
            } else {
                break Optioned::<usize>::none();
            };

            if elem.is(needle) {
                f();
                if SHORT {
                    break Optioned::<usize>::some(i);
                }
                borrower = Some(guard);
            } else {
                let elem = elem.clone();
                drop(guard);

                if elem.rich_compare_bool(needle, PyComparisonOp::Eq, vm)? {
                    f();
                    if SHORT {
                        break Optioned::<usize>::some(i);
                    }
                }
            }
            i += 1;
        };

        Ok(index)
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
