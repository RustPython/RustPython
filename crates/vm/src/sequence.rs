use crate::{
    AsObject, PyObject, PyObjectRef, PyResult,
    function::OptionalArg,
    sliceable::to_saturated_index,
    types::PyComparisonOp,
    vm::{MAX_MEMORY_SIZE, VirtualMachine},
};
use core::ops::{Deref, Range};
use optional::Optioned;

pub trait MutObjectSequenceOp {
    type Inner: ?Sized;

    fn do_get(index: usize, inner: &Self::Inner) -> Option<&PyObject>;
    fn do_lock(&self) -> impl Deref<Target = Self::Inner>;

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
                let elem = elem.to_owned();
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
        if self.as_ref().is_empty() {
            return Ok(Vec::new());
        }

        if n > 1 && core::mem::size_of_val(self.as_ref()) >= MAX_MEMORY_SIZE / n {
            return Err(vm.no_memory_error());
        }

        let total = n
            .checked_mul(self.as_ref().len())
            .ok_or_else(|| vm.no_memory_error())?;
        let mut v = Vec::new();
        v.try_reserve_exact(total)
            .map_err(|_| vm.no_memory_error())?;
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

        if n > 1 && core::mem::size_of_val(self.as_ref()) >= MAX_MEMORY_SIZE / n {
            // TODO: make a global static NoMemory shared exc object and return its reference.
            return Err(vm.no_memory_error());
        }

        if n == 0 {
            self.as_vec_mut().clear();
        } else if n != 1 && !self.as_ref().is_empty() {
            let len = self.as_ref().len();
            let v = self.as_vec_mut();
            v.try_reserve_exact(len * (n - 1))
                .map_err(|_| vm.no_memory_error())?;
            for _ in 1..n {
                v.extend_from_within(..len);
            }
        }
        Ok(())
    }
}

impl<T: Clone> SequenceMutExt<T> for Vec<T> {
    fn as_vec_mut(&mut self) -> &mut Self {
        self
    }
}

#[derive(FromArgs)]
pub struct OptionalRangeArgs {
    #[pyarg(positional, default = 0)]
    start: PyObjectRef,
    // Platform ssize maximum; a growing list may reach beyond its initial length.
    #[pyarg(positional, optional, py_default = "sys.maxsize")]
    stop: OptionalArg<PyObjectRef>,
}

impl Drop for OptionalRangeArgs {
    fn drop(&mut self) {
        // Call arguments are released right to left after conversion and search.
        // Keep both bounds alive until then, but release stop before start.
        drop(core::mem::replace(&mut self.stop, OptionalArg::Missing));
    }
}

impl OptionalRangeArgs {
    pub fn index_range(
        &self,
        len: impl FnOnce() -> usize,
        vm: &VirtualMachine,
    ) -> PyResult<Range<usize>> {
        let convert = |obj: &PyObject| {
            to_saturated_index(
                vm,
                obj,
                "slice indices must be integers or have an __index__ method",
            )
        };
        let start = convert(&self.start)?;
        let stop = match &self.stop {
            OptionalArg::Present(stop) => convert(stop)?,
            OptionalArg::Missing => isize::MAX,
        };
        // Both conversions may mutate the list. Normalize negative bounds only
        // after they finish, and leave positive bounds open to later growth.
        let len = len() as isize;
        let normalize = |index: isize| {
            if index < 0 {
                index.saturating_add(len).max(0) as usize
            } else {
                index as usize
            }
        };
        Ok(normalize(start)..normalize(stop))
    }
}
