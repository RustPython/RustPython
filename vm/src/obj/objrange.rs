use std::cell::Cell;
use std::ops::Mul;

use num_bigint::{BigInt, Sign};
use num_integer::Integer;
use num_traits::{One, Signed, ToPrimitive, Zero};

use crate::function::{OptionalArg, PyFuncArgs};
use crate::pyobject::{
    PyContext, PyIteratorValue, PyObjectRef, PyRef, PyResult, PyValue, TypeProtocol,
};
use crate::vm::VirtualMachine;

use super::objint::{self, PyInt, PyIntRef};
use super::objslice::PySlice;
use super::objtype;
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
    pub fn try_len(&self) -> Option<usize> {
        match self.step.sign() {
            Sign::Plus if self.start < self.stop => ((&self.stop - &self.start - 1usize)
                / &self.step)
                .to_usize()
                .map(|sz| sz + 1),
            Sign::Minus if self.start > self.stop => ((&self.start - &self.stop - 1usize)
                / (-&self.step))
                .to_usize()
                .map(|sz| sz + 1),
            _ => Some(0),
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.try_len().unwrap()
    }

    #[inline]
    fn offset(&self, value: &BigInt) -> Option<BigInt> {
        match self.step.sign() {
            Sign::Plus if *value >= self.start && *value < self.stop => Some(value - &self.start),
            Sign::Minus if *value <= self.start && *value > self.stop => Some(&self.start - value),
            _ => None,
        }
    }

    #[inline]
    pub fn contains(&self, value: &BigInt) -> bool {
        match self.offset(value) {
            Some(ref offset) => offset.is_multiple_of(&self.step),
            None => false,
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
    pub fn count(&self, value: &BigInt) -> usize {
        if self.index_of(value).is_some() {
            1
        } else {
            0
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

    #[inline]
    pub fn reversed(&self) -> Self {
        // compute the last element that is actually contained within the range
        // this is the new start
        let remainder = ((&self.stop - &self.start) % &self.step).abs();
        let start = if remainder.is_zero() {
            &self.stop - &self.step
        } else {
            &self.stop - &remainder
        };

        match self.step.sign() {
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
        }
    }

    pub fn repr(&self) -> String {
        if self.step == BigInt::one() {
            format!("range({}, {})", self.start, self.stop)
        } else {
            format!("range({}, {}, {})", self.start, self.stop, self.step)
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
        "__bool__" => context.new_rustfunc(range_bool),
        "__contains__" => context.new_rustfunc(range_contains),
        "__doc__" => context.new_str(range_doc.to_string()),
        "__getitem__" => context.new_rustfunc(range_getitem),
        "__iter__" => context.new_rustfunc(range_iter),
        "__len__" => context.new_rustfunc(range_len),
        "__new__" => context.new_rustfunc(range_new),
        "__repr__" => context.new_rustfunc(range_repr),
        "__reversed__" => context.new_rustfunc(range_reversed),
        "count" => context.new_rustfunc(range_count),
        "index" => context.new_rustfunc(range_index),
        "start" => context.new_property(range_start),
        "step" => context.new_property(range_step),
        "stop" => context.new_property(range_stop)
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

fn range_iter(range: PyRangeRef, _vm: &VirtualMachine) -> PyIteratorValue {
    PyIteratorValue {
        position: Cell::new(0),
        iterated_obj: range.into_object(),
    }
}

fn range_reversed(zelf: PyRangeRef, vm: &VirtualMachine) -> PyIteratorValue {
    let range = zelf.reversed();

    PyIteratorValue {
        position: Cell::new(0),
        iterated_obj: range.into_ref(vm).into_object(),
    }
}

fn range_len(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, Some(vm.ctx.range_type()))]);

    if let Some(len) = get_value(zelf).try_len() {
        Ok(vm.ctx.new_int(len))
    } else {
        Err(vm.new_overflow_error("Python int too large to convert to Rust usize".to_string()))
    }
}

fn range_getitem(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.range_type())), (subscript, None)]
    );

    let range = get_value(zelf);

    if let Some(i) = subscript.payload::<PyInt>() {
        if let Some(int) = range.get(i.value.clone()) {
            Ok(vm.ctx.new_int(int))
        } else {
            Err(vm.new_index_error("range object index out of range".to_string()))
        }
    } else if let Some(PySlice {
        ref start,
        ref stop,
        ref step,
    }) = subscript.payload()
    {
        let new_start = if let Some(int) = start {
            if let Some(i) = range.get(int) {
                i
            } else {
                range.start.clone()
            }
        } else {
            range.start.clone()
        };

        let new_end = if let Some(int) = stop {
            if let Some(i) = range.get(int) {
                i
            } else {
                range.stop
            }
        } else {
            range.stop
        };

        let new_step = if let Some(int) = step {
            int * range.step
        } else {
            range.step
        };

        Ok(PyRange {
            start: new_start,
            stop: new_end,
            step: new_step,
        }
        .into_ref(vm)
        .into_object())
    } else {
        Err(vm.new_type_error("range indices must be integer or slice".to_string()))
    }
}

fn range_repr(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, Some(vm.ctx.range_type()))]);

    let repr = get_value(zelf).repr();

    Ok(vm.ctx.new_str(repr))
}

fn range_bool(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, Some(vm.ctx.range_type()))]);

    let len = get_value(zelf).len();

    Ok(vm.ctx.new_bool(len > 0))
}

fn range_contains(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.range_type())), (needle, None)]
    );

    let range = get_value(zelf);

    let result = if objtype::isinstance(needle, &vm.ctx.int_type()) {
        range.contains(&objint::get_value(needle))
    } else {
        false
    };

    Ok(vm.ctx.new_bool(result))
}

fn range_index(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.range_type())), (needle, None)]
    );

    let range = get_value(zelf);

    if objtype::isinstance(needle, &vm.ctx.int_type()) {
        let needle = objint::get_value(needle);

        match range.index_of(&needle) {
            Some(idx) => Ok(vm.ctx.new_int(idx)),
            None => Err(vm.new_value_error(format!("{} is not in range", needle))),
        }
    } else {
        Err(vm.new_value_error("sequence.index(x): x not in sequence".to_string()))
    }
}

fn range_count(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.range_type())), (item, None)]
    );

    let range = get_value(zelf);

    if objtype::isinstance(item, &vm.ctx.int_type()) {
        Ok(vm.ctx.new_int(range.count(&objint::get_value(item))))
    } else {
        Ok(vm.ctx.new_int(0))
    }
}

fn range_start(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, Some(vm.ctx.range_type()))]);
    Ok(vm.ctx.new_int(get_value(zelf).start))
}

fn range_stop(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, Some(vm.ctx.range_type()))]);
    Ok(vm.ctx.new_int(get_value(zelf).stop))
}

fn range_step(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, Some(vm.ctx.range_type()))]);
    Ok(vm.ctx.new_int(get_value(zelf).step))
}
