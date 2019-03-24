use std::cell::Cell;
use std::ops::Mul;

use num_bigint::{BigInt, Sign};
use num_integer::Integer;
use num_traits::{One, Signed, Zero};

use crate::function::{OptionalArg, PyFuncArgs};
use crate::pyobject::{Either, PyContext, PyIteratorValue, PyObjectRef, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

use super::objint::{PyInt, PyIntRef};
use super::objslice::PySliceRef;
use super::objtype::PyClassRef;

#[derive(Debug, Clone)]
pub struct PyRange {
    // Unfortunately Rust's built in range type doesn't support things like indexing
    // or ranges where start > end so we need to roll our own.
    pub start: BigInt,
    pub stop: BigInt,
    pub step: BigInt,
}

impl PyValue for PyRange {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.range_type()
    }
}

impl PyRange {
    #[inline]
    fn offset(&self, value: &BigInt) -> Option<BigInt> {
        match self.step.sign() {
            Sign::Plus if *value >= self.start && *value < self.stop => Some(value - &self.start),
            Sign::Minus if *value <= self.start && *value > self.stop => Some(&self.start - value),
            _ => None,
        }
    }

    #[inline]
    pub fn index_of(&self, value: &BigInt) -> Option<BigInt> {
        match self.offset(value) {
            Some(ref offset) if offset.is_multiple_of(&self.step) => {
                Some((offset / &self.step).abs())
            }
            Some(_) | None => None,
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        (self.start <= self.stop && self.step.is_negative())
            || (self.start >= self.stop && self.step.is_positive())
    }

    #[inline]
    pub fn forward(&self) -> bool {
        self.start < self.stop
    }

    #[inline]
    pub fn get<'a, T>(&'a self, index: T) -> Option<BigInt>
    where
        &'a BigInt: Mul<T, Output = BigInt>,
    {
        let result = &self.start + &self.step * index;

        if (self.forward() && !self.is_empty() && result < self.stop)
            || (!self.forward() && !self.is_empty() && result > self.stop)
        {
            Some(result)
        } else {
            None
        }
    }
}

pub fn get_value(obj: &PyObjectRef) -> PyRange {
    obj.payload::<PyRange>().unwrap().clone()
}

pub fn init(context: &PyContext) {
    let range_type = &context.range_type;

    let range_doc = "range(stop) -> range object\n\
                     range(start, stop[, step]) -> range object\n\n\
                     Return an object that produces a sequence of integers from start (inclusive)\n\
                     to stop (exclusive) by step.  range(i, j) produces i, i+1, i+2, ..., j-1.\n\
                     start defaults to 0, and stop is omitted!  range(4) produces 0, 1, 2, 3.\n\
                     These are exactly the valid indices for a list of 4 elements.\n\
                     When step is given, it specifies the increment (or decrement).";

    extend_class!(context, range_type, {
        "__bool__" => context.new_rustfunc(PyRangeRef::bool),
        "__contains__" => context.new_rustfunc(PyRangeRef::contains),
        "__doc__" => context.new_str(range_doc.to_string()),
        "__getitem__" => context.new_rustfunc(PyRangeRef::getitem),
        "__iter__" => context.new_rustfunc(PyRangeRef::iter),
        "__len__" => context.new_rustfunc(PyRangeRef::len),
        "__new__" => context.new_rustfunc(range_new),
        "__repr__" => context.new_rustfunc(PyRangeRef::repr),
        "__reversed__" => context.new_rustfunc(PyRangeRef::reversed),
        "count" => context.new_rustfunc(PyRangeRef::count),
        "index" => context.new_rustfunc(PyRangeRef::index),
        "start" => context.new_property(PyRangeRef::start),
        "stop" => context.new_property(PyRangeRef::stop),
        "step" => context.new_property(PyRangeRef::step),
    });
}

type PyRangeRef = PyRef<PyRange>;

impl PyRangeRef {
    fn new(cls: PyClassRef, stop: PyIntRef, vm: &VirtualMachine) -> PyResult<PyRangeRef> {
        PyRange {
            start: Zero::zero(),
            stop: stop.value.clone(),
            step: One::one(),
        }
        .into_ref_with_type(vm, cls)
    }

    fn new_from(
        cls: PyClassRef,
        start: PyIntRef,
        stop: PyIntRef,
        step: OptionalArg<PyIntRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyRangeRef> {
        PyRange {
            start: start.value.clone(),
            stop: stop.value.clone(),
            step: step
                .into_option()
                .map(|i| i.value.clone())
                .unwrap_or_else(One::one),
        }
        .into_ref_with_type(vm, cls)
    }

    fn start(self, _vm: &VirtualMachine) -> BigInt {
        self.start.clone()
    }

    fn stop(self, _vm: &VirtualMachine) -> BigInt {
        self.stop.clone()
    }

    fn step(self, _vm: &VirtualMachine) -> BigInt {
        self.step.clone()
    }

    fn iter(self: PyRangeRef, _vm: &VirtualMachine) -> PyIteratorValue {
        PyIteratorValue {
            position: Cell::new(0),
            iterated_obj: self.into_object(),
        }
    }

    fn reversed(self: PyRangeRef, vm: &VirtualMachine) -> PyIteratorValue {
        // compute the last element that is actually contained within the range
        // this is the new start
        let remainder = ((&self.stop - &self.start) % &self.step).abs();
        let start = if remainder.is_zero() {
            &self.stop - &self.step
        } else {
            &self.stop - &remainder
        };

        let reversed = match self.step.sign() {
            Sign::Plus => PyRange {
                start,
                stop: &self.start - 1,
                step: -&self.step,
            },
            Sign::Minus => PyRange {
                start,
                stop: &self.start + 1,
                step: -&self.step,
            },
            Sign::NoSign => unreachable!(),
        };
        PyIteratorValue {
            position: Cell::new(0),
            iterated_obj: reversed.into_ref(vm).into_object(),
        }
    }

    fn len(self, _vm: &VirtualMachine) -> PyInt {
        match self.step.sign() {
            Sign::Plus if self.start < self.stop => {
                PyInt::new((&self.stop - &self.start - 1usize) / &self.step + 1)
            }
            Sign::Minus if self.start > self.stop => {
                PyInt::new((&self.start - &self.stop - 1usize) / (-&self.step) + 1)
            }
            Sign::Plus | Sign::Minus => PyInt::new(0),
            Sign::NoSign => unreachable!(),
        }
    }

    fn repr(self, _vm: &VirtualMachine) -> String {
        if self.step.is_one() {
            format!("range({}, {})", self.start, self.stop)
        } else {
            format!("range({}, {}, {})", self.start, self.stop, self.step)
        }
    }

    fn bool(self, _vm: &VirtualMachine) -> bool {
        !self.is_empty()
    }

    fn contains(self, needle: PyObjectRef, _vm: &VirtualMachine) -> bool {
        if let Ok(int) = needle.downcast::<PyInt>() {
            match self.offset(&int.value) {
                Some(ref offset) => offset.is_multiple_of(&self.step),
                None => false,
            }
        } else {
            false
        }
    }

    fn index(self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyInt> {
        if let Ok(int) = needle.downcast::<PyInt>() {
            match self.index_of(&int.value) {
                Some(idx) => Ok(PyInt::new(idx)),
                None => Err(vm.new_value_error(format!("{} is not in range", int))),
            }
        } else {
            Err(vm.new_value_error("sequence.index(x): x not in sequence".to_string()))
        }
    }

    fn count(self, item: PyObjectRef, _vm: &VirtualMachine) -> PyInt {
        if let Ok(int) = item.downcast::<PyInt>() {
            if self.index_of(&int.value).is_some() {
                PyInt::new(1)
            } else {
                PyInt::new(0)
            }
        } else {
            PyInt::new(0)
        }
    }

    fn getitem(self, subscript: Either<PyIntRef, PySliceRef>, vm: &VirtualMachine) -> PyResult {
        match subscript {
            Either::A(index) => {
                if let Some(value) = self.get(index.value.clone()) {
                    Ok(PyInt::new(value).into_ref(vm).into_object())
                } else {
                    Err(vm.new_index_error("range object index out of range".to_string()))
                }
            }
            Either::B(slice) => {
                let new_start = if let Some(int) = slice.start.clone() {
                    if let Some(i) = self.get(int) {
                        i
                    } else {
                        self.start.clone()
                    }
                } else {
                    self.start.clone()
                };

                let new_end = if let Some(int) = slice.stop.clone() {
                    if let Some(i) = self.get(int) {
                        i
                    } else {
                        self.stop.clone()
                    }
                } else {
                    self.stop.clone()
                };

                let new_step = if let Some(int) = slice.step.clone() {
                    int * self.step.clone()
                } else {
                    self.step.clone()
                };

                Ok(PyRange {
                    start: new_start,
                    stop: new_end,
                    step: new_step,
                }
                .into_ref(vm)
                .into_object())
            }
        }
    }
}

fn range_new(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    let range = if args.args.len() <= 2 {
        let (cls, stop) = args.bind(vm)?;
        PyRangeRef::new(cls, stop, vm)
    } else {
        let (cls, start, stop, step) = args.bind(vm)?;
        PyRangeRef::new_from(cls, start, stop, step, vm)
    }?;

    Ok(range.into_object())
}
