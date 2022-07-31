use crate::{
    builtins::PyIntRef,
    function::{Either, OptionalArg, PyComparisonValue},
    sliceable::SequenceIndexOp,
    types::{richcompare_wrapper, PyComparisonOp, RichCompareFunc},
    vm::VirtualMachine,
    AsObject, PyObject, PyObjectRef, PyResult,
};
use optional::Optioned;
use std::ops::Range;

pub trait MutObjectSequenceOp<'a> {
    type Guard;

    fn do_get(index: usize, guard: &Self::Guard) -> Option<&PyObjectRef>;
    fn do_lock(&'a self) -> Self::Guard;

    fn mut_count(&'a self, vm: &VirtualMachine, needle: &PyObject) -> PyResult<usize> {
        let mut count = 0;
        self._mut_iter_equal_skeleton::<_, false>(vm, needle, 0..isize::MAX as usize, || {
            count += 1
        })?;
        Ok(count)
    }

    fn mut_index_range(
        &'a self,
        vm: &VirtualMachine,
        needle: &PyObject,
        range: Range<usize>,
    ) -> PyResult<Optioned<usize>> {
        self._mut_iter_equal_skeleton::<_, true>(vm, needle, range, || {})
    }

    fn mut_index(&'a self, vm: &VirtualMachine, needle: &PyObject) -> PyResult<Optioned<usize>> {
        self.mut_index_range(vm, needle, 0..isize::MAX as usize)
    }

    fn mut_contains(&'a self, vm: &VirtualMachine, needle: &PyObject) -> PyResult<bool> {
        self.mut_index(vm, needle).map(|x| x.is_some())
    }

    fn _mut_iter_equal_skeleton<F, const SHORT: bool>(
        &'a self,
        vm: &VirtualMachine,
        needle: &PyObject,
        range: Range<usize>,
        mut f: F,
    ) -> PyResult<Optioned<usize>>
    where
        F: FnMut(),
    {
        let needle_cls = needle.class();
        let needle_cmp = needle_cls
            .mro_find_map(|cls| cls.slots.richcompare.load())
            .unwrap();

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
                let elem_cls = elem.class();
                let reverse_first =
                    !elem_cls.is(&needle_cls) && elem_cls.fast_issubclass(&needle_cls);

                let eq = if reverse_first {
                    let elem_cmp = elem_cls
                        .mro_find_map(|cls| cls.slots.richcompare.load())
                        .unwrap();
                    drop(elem_cls);

                    fn cmp(
                        elem: &PyObject,
                        needle: &PyObject,
                        elem_cmp: RichCompareFunc,
                        needle_cmp: RichCompareFunc,
                        vm: &VirtualMachine,
                    ) -> PyResult<bool> {
                        match elem_cmp(elem, needle, PyComparisonOp::Eq, vm)? {
                            Either::B(PyComparisonValue::Implemented(value)) => Ok(value),
                            Either::A(obj) if !obj.is(&vm.ctx.not_implemented) => {
                                obj.try_to_bool(vm)
                            }
                            _ => match needle_cmp(needle, elem, PyComparisonOp::Eq, vm)? {
                                Either::B(PyComparisonValue::Implemented(value)) => Ok(value),
                                Either::A(obj) if !obj.is(&vm.ctx.not_implemented) => {
                                    obj.try_to_bool(vm)
                                }
                                _ => Ok(false),
                            },
                        }
                    }

                    if elem_cmp as usize == richcompare_wrapper as usize {
                        let elem = elem.clone();
                        drop(guard);
                        cmp(&elem, needle, elem_cmp, needle_cmp, vm)?
                    } else {
                        let eq = cmp(elem, needle, elem_cmp, needle_cmp, vm)?;
                        borrower = Some(guard);
                        eq
                    }
                } else {
                    match needle_cmp(needle, elem, PyComparisonOp::Eq, vm)? {
                        Either::B(PyComparisonValue::Implemented(value)) => {
                            drop(elem_cls);
                            borrower = Some(guard);
                            value
                        }
                        Either::A(obj) if !obj.is(&vm.ctx.not_implemented) => {
                            drop(elem_cls);
                            borrower = Some(guard);
                            obj.try_to_bool(vm)?
                        }
                        _ => {
                            let elem_cmp = elem_cls
                                .mro_find_map(|cls| cls.slots.richcompare.load())
                                .unwrap();
                            drop(elem_cls);

                            fn cmp(
                                elem: &PyObject,
                                needle: &PyObject,
                                elem_cmp: RichCompareFunc,
                                vm: &VirtualMachine,
                            ) -> PyResult<bool> {
                                match elem_cmp(elem, needle, PyComparisonOp::Eq, vm)? {
                                    Either::B(PyComparisonValue::Implemented(value)) => Ok(value),
                                    Either::A(obj) if !obj.is(&vm.ctx.not_implemented) => {
                                        obj.try_to_bool(vm)
                                    }
                                    _ => Ok(false),
                                }
                            }

                            if elem_cmp as usize == richcompare_wrapper as usize {
                                let elem = elem.clone();
                                drop(guard);
                                cmp(&elem, needle, elem_cmp, vm)?
                            } else {
                                let eq = cmp(elem, needle, elem_cmp, vm)?;
                                borrower = Some(guard);
                                eq
                            }
                        }
                    }
                };

                if eq {
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
