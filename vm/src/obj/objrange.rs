use std::cell::Cell;

use num_bigint::{BigInt, Sign};
use num_integer::Integer;
use num_traits::{One, Signed, Zero};

use crate::function::{OptionalArg, PyFuncArgs};
use crate::pyobject::{
    PyContext, PyObjectRef, PyRef, PyResult, PyValue, TryFromObject, TypeProtocol,
};
use crate::vm::VirtualMachine;

use super::objint::{PyInt, PyIntRef};
use super::objiter;
use super::objslice::{PySlice, PySliceRef};
use super::objtype::{self, PyClassRef};

#[derive(Debug, Clone)]
pub struct PyRange {
    pub start: PyIntRef,
    pub stop: PyIntRef,
    pub step: PyIntRef,
}

impl PyValue for PyRange {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.range_type()
    }
}

impl PyRange {
    #[inline]
    fn offset(&self, value: &BigInt) -> Option<BigInt> {
        let start = self.start.as_bigint();
        let stop = self.stop.as_bigint();
        let step = self.step.as_bigint();
        match step.sign() {
            Sign::Plus if value >= start && value < stop => Some(value - start),
            Sign::Minus if value <= self.start.as_bigint() && value > stop => Some(start - value),
            _ => None,
        }
    }

    #[inline]
    pub fn index_of(&self, value: &BigInt) -> Option<BigInt> {
        let step = self.step.as_bigint();
        match self.offset(value) {
            Some(ref offset) if offset.is_multiple_of(step) => Some((offset / step).abs()),
            Some(_) | None => None,
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        let start = self.start.as_bigint();
        let stop = self.stop.as_bigint();
        let step = self.step.as_bigint();
        (start <= stop && step.is_negative()) || (start >= stop && step.is_positive())
    }

    #[inline]
    pub fn forward(&self) -> bool {
        self.start.as_bigint() < self.stop.as_bigint()
    }

    #[inline]
    pub fn get(&self, index: &BigInt) -> Option<BigInt> {
        let start = self.start.as_bigint();
        let stop = self.stop.as_bigint();
        let step = self.step.as_bigint();

        let index = if index < &BigInt::zero() {
            let index = stop + index;
            if index < BigInt::zero() {
                return None;
            }
            index
        } else {
            index.clone()
        };

        let result = start + step * &index;

        if (self.forward() && !self.is_empty() && &result < stop)
            || (!self.forward() && !self.is_empty() && &result > stop)
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
        "__bool__" => context.new_rustfunc(PyRange::bool),
        "__contains__" => context.new_rustfunc(PyRange::contains),
        "__doc__" => context.new_str(range_doc.to_string()),
        "__eq__" => context.new_rustfunc(PyRange::eq),
        "__getitem__" => context.new_rustfunc(PyRange::getitem),
        "__iter__" => context.new_rustfunc(PyRange::iter),
        "__len__" => context.new_rustfunc(PyRange::len),
        "__new__" => context.new_rustfunc(range_new),
        "__repr__" => context.new_rustfunc(PyRange::repr),
        "__reversed__" => context.new_rustfunc(PyRange::reversed),
        "count" => context.new_rustfunc(PyRange::count),
        "index" => context.new_rustfunc(PyRange::index),
        "start" => context.new_property(PyRange::start),
        "stop" => context.new_property(PyRange::stop),
        "step" => context.new_property(PyRange::step),
    });

    let rangeiterator_type = &context.rangeiterator_type;
    extend_class!(context, rangeiterator_type, {
        "__next__" => context.new_rustfunc(PyRangeIteratorRef::next),
        "__iter__" => context.new_rustfunc(PyRangeIteratorRef::iter),
    });
}

type PyRangeRef = PyRef<PyRange>;

impl PyRange {
    fn new(cls: PyClassRef, stop: PyIntRef, vm: &VirtualMachine) -> PyResult<PyRangeRef> {
        PyRange {
            start: PyInt::new(BigInt::zero()).into_ref(vm),
            stop: stop.clone(),
            step: PyInt::new(BigInt::one()).into_ref(vm),
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
            start,
            stop,
            step: step
                .into_option()
                .unwrap_or_else(|| PyInt::new(BigInt::one()).into_ref(vm)),
        }
        .into_ref_with_type(vm, cls)
    }

    fn start(&self, _vm: &VirtualMachine) -> PyIntRef {
        self.start.clone()
    }

    fn stop(&self, _vm: &VirtualMachine) -> PyIntRef {
        self.stop.clone()
    }

    fn step(&self, _vm: &VirtualMachine) -> PyIntRef {
        self.step.clone()
    }

    fn iter(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyRangeIterator {
        PyRangeIterator {
            position: Cell::new(0),
            range: zelf,
        }
    }

    fn reversed(&self, vm: &VirtualMachine) -> PyRangeIterator {
        let start = self.start.as_bigint();
        let stop = self.stop.as_bigint();
        let step = self.step.as_bigint();

        // compute the last element that is actually contained within the range
        // this is the new start
        let remainder = ((stop - start) % step).abs();
        let new_start = if remainder.is_zero() {
            stop - step
        } else {
            stop - &remainder
        };

        let new_stop: BigInt = match step.sign() {
            Sign::Plus => start - 1,
            Sign::Minus => start + 1,
            Sign::NoSign => unreachable!(),
        };

        let reversed = PyRange {
            start: PyInt::new(new_start).into_ref(vm),
            stop: PyInt::new(new_stop).into_ref(vm),
            step: PyInt::new(-step).into_ref(vm),
        };

        PyRangeIterator {
            position: Cell::new(0),
            range: reversed.into_ref(vm),
        }
    }

    fn len(&self, _vm: &VirtualMachine) -> PyInt {
        let start = self.start.as_bigint();
        let stop = self.stop.as_bigint();
        let step = self.step.as_bigint();

        match step.sign() {
            Sign::Plus if start < stop => PyInt::new((stop - start - 1usize) / step + 1),
            Sign::Minus if start > stop => PyInt::new((start - stop - 1usize) / (-step) + 1),
            Sign::Plus | Sign::Minus => PyInt::new(0),
            Sign::NoSign => unreachable!(),
        }
    }

    fn repr(&self, _vm: &VirtualMachine) -> String {
        if self.step.as_bigint().is_one() {
            format!("range({}, {})", self.start, self.stop)
        } else {
            format!("range({}, {}, {})", self.start, self.stop, self.step)
        }
    }

    fn bool(&self, _vm: &VirtualMachine) -> bool {
        !self.is_empty()
    }

    fn contains(&self, needle: PyObjectRef, _vm: &VirtualMachine) -> bool {
        if let Ok(int) = needle.downcast::<PyInt>() {
            match self.offset(int.as_bigint()) {
                Some(ref offset) => offset.is_multiple_of(self.step.as_bigint()),
                None => false,
            }
        } else {
            false
        }
    }

    fn eq(&self, rhs: PyObjectRef, vm: &VirtualMachine) -> bool {
        if objtype::isinstance(&rhs, &vm.ctx.range_type()) {
            let rhs = get_value(&rhs);
            self.start.as_bigint() == rhs.start.as_bigint()
                && self.stop.as_bigint() == rhs.stop.as_bigint()
                && self.step.as_bigint() == rhs.step.as_bigint()
        } else {
            false
        }
    }

    fn index(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyInt> {
        if let Ok(int) = needle.downcast::<PyInt>() {
            match self.index_of(int.as_bigint()) {
                Some(idx) => Ok(PyInt::new(idx)),
                None => Err(vm.new_value_error(format!("{} is not in range", int))),
            }
        } else {
            Err(vm.new_value_error("sequence.index(x): x not in sequence".to_string()))
        }
    }

    fn count(&self, item: PyObjectRef, _vm: &VirtualMachine) -> PyInt {
        if let Ok(int) = item.downcast::<PyInt>() {
            if self.index_of(int.as_bigint()).is_some() {
                PyInt::new(1)
            } else {
                PyInt::new(0)
            }
        } else {
            PyInt::new(0)
        }
    }

    fn getitem(&self, subscript: RangeIndex, vm: &VirtualMachine) -> PyResult {
        match subscript {
            RangeIndex::Int(index) => {
                if let Some(value) = self.get(index.as_bigint()) {
                    Ok(PyInt::new(value).into_ref(vm).into_object())
                } else {
                    Err(vm.new_index_error("range object index out of range".to_string()))
                }
            }
            RangeIndex::Slice(slice) => {
                let new_start = if let Some(int) = slice.start_index(vm)? {
                    if let Some(i) = self.get(&int) {
                        PyInt::new(i).into_ref(vm)
                    } else {
                        self.start.clone()
                    }
                } else {
                    self.start.clone()
                };

                let new_end = if let Some(int) = slice.stop_index(vm)? {
                    if let Some(i) = self.get(&int) {
                        PyInt::new(i).into_ref(vm)
                    } else {
                        self.stop.clone()
                    }
                } else {
                    self.stop.clone()
                };

                let new_step = if let Some(int) = slice.step_index(vm)? {
                    PyInt::new(int * self.step.as_bigint()).into_ref(vm)
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
        PyRange::new(cls, stop, vm)
    } else {
        let (cls, start, stop, step) = args.bind(vm)?;
        PyRange::new_from(cls, start, stop, step, vm)
    }?;

    Ok(range.into_object())
}

#[derive(Debug)]
pub struct PyRangeIterator {
    position: Cell<usize>,
    range: PyRangeRef,
}

impl PyValue for PyRangeIterator {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.rangeiterator_type()
    }
}

type PyRangeIteratorRef = PyRef<PyRangeIterator>;

impl PyRangeIteratorRef {
    fn next(self, vm: &VirtualMachine) -> PyResult<BigInt> {
        let position = BigInt::from(self.position.get());
        if let Some(int) = self.range.get(&position) {
            self.position.set(self.position.get() + 1);
            Ok(int)
        } else {
            Err(objiter::new_stop_iteration(vm))
        }
    }

    fn iter(self, _vm: &VirtualMachine) -> Self {
        self
    }
}

pub enum RangeIndex {
    Int(PyIntRef),
    Slice(PySliceRef),
}

impl TryFromObject for RangeIndex {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        match_class!(obj,
            i @ PyInt => Ok(RangeIndex::Int(i)),
            s @ PySlice => Ok(RangeIndex::Slice(s)),
            obj => Err(vm.new_type_error(format!(
                "sequence indices be integers or slices, not {}",
                obj.class(),
            )))
        )
    }
}
