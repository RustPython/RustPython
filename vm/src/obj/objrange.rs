use super::super::pyobject::{
    PyContext, PyFuncArgs, PyObject, PyObjectPayload, PyObjectRef, PyResult, TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objint;
use super::objtype;
use num_bigint::BigInt;
use num_integer::Integer;
use num_traits::{One, Signed, ToPrimitive, Zero};
use std::cell::Ref;

#[derive(Debug, Clone)]
pub struct RangeType {
    // Unfortunately Rust's built in range type doesn't support things like indexing
    // or ranges where start > end so we need to roll our own.
    pub start: BigInt,
    pub end: BigInt,
    pub step: BigInt,
}

impl RangeType {
    #[inline]
    pub fn len(&self) -> usize {
        ((self.end.clone() - self.start.clone()) / self.step.clone())
            .abs()
            .to_usize()
            .unwrap()
    }

    #[inline]
    pub fn index_of(&self, value: &BigInt) -> Option<BigInt> {
        if value < &self.start || value >= &self.end {
            return None;
        }

        let offset = value - &self.start;
        if offset.is_multiple_of(&self.step) {
            Some(offset / &self.step)
        } else {
            None
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        (self.start <= self.end && self.step.is_negative())
            || (self.start >= self.end && self.step.is_positive())
    }

    #[inline]
    pub fn forward(&self) -> bool {
        self.start < self.end
    }

    #[inline]
    pub fn get(&self, index: BigInt) -> Option<BigInt> {
        let result = self.start.clone() + self.step.clone() * index;

        if self.forward() && !self.is_empty() && result < self.end {
            Some(result)
        } else if !self.forward() && !self.is_empty() && result > self.end {
            Some(result)
        } else {
            None
        }
    }
}

pub fn init(context: &PyContext) {
    let ref range_type = context.range_type;
    context.set_attr(&range_type, "__new__", context.new_rustfunc(range_new));
    context.set_attr(&range_type, "__bool__", context.new_rustfunc(range_bool));
    context.set_attr(&range_type, "__iter__", context.new_rustfunc(range_iter));
    context.set_attr(&range_type, "__len__", context.new_rustfunc(range_len));
    context.set_attr(
        &range_type,
        "__getitem__",
        context.new_rustfunc(range_getitem),
    );
    context.set_attr(&range_type, "index", context.new_rustfunc(range_index));
}

fn range_new(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(cls, None), (first, Some(vm.ctx.int_type()))],
        optional = [
            (second, Some(vm.ctx.int_type())),
            (step, Some(vm.ctx.int_type()))
        ]
    );

    let start = if let Some(_) = second {
        objint::get_value(first)
    } else {
        BigInt::zero()
    };

    let end = if let Some(pyint) = second {
        objint::get_value(pyint)
    } else {
        objint::get_value(first)
    };

    let step = if let Some(pyint) = step {
        objint::get_value(pyint)
    } else {
        BigInt::one()
    };

    if step.is_zero() {
        Err(vm.new_value_error("range with 0 step size".to_string()))
    } else {
        Ok(PyObject::new(
            PyObjectPayload::Range {
                range: RangeType { start, end, step },
            },
            cls.clone(),
        ))
    }
}

fn range_iter(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(range, Some(vm.ctx.range_type()))]);

    Ok(PyObject::new(
        PyObjectPayload::Iterator {
            position: 0,
            iterated_obj: range.clone(),
        },
        vm.ctx.iter_type(),
    ))
}

fn range_len(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, Some(vm.ctx.range_type()))]);
    let len = get_value(zelf).len();
    Ok(vm.ctx.new_int(len))
}

fn range_getitem(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.range_type())), (subscript, None)]
    );
    let zrange = get_value(zelf).clone();

    match subscript.borrow().payload {
        PyObjectPayload::Integer { ref value } => {
            if let Some(int) = zrange.get(value.clone()) {
                Ok(vm.ctx.new_int(int))
            } else {
                Err(vm.new_index_error("range object index out of range".to_string()))
            }
        }
        PyObjectPayload::Slice { start, stop, step } => {
            let new_start = if let Some(int) = start {
                if let Some(i) = zrange.get(int.into()) {
                    i
                } else {
                    zrange.start.clone()
                }
            } else {
                zrange.start.clone()
            };

            let new_end = if let Some(int) = stop {
                if let Some(i) = zrange.get(int.into()) {
                    i
                } else {
                    zrange.end
                }
            } else {
                zrange.end
            };

            let new_step = if let Some(int) = step {
                (int as i64) * zrange.step
            } else {
                zrange.step
            };

            Ok(PyObject::new(
                PyObjectPayload::Range {
                    range: RangeType {
                        start: new_start,
                        end: new_end,
                        step: new_step,
                    },
                },
                vm.ctx.range_type(),
            ))
        }

        _ => Err(vm.new_type_error("range indices must be integer or slice".to_string())),
    }
}

fn range_index(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.range_type())), (needle, None)]
    );

    let range = get_value(zelf);
    match range.index_of(&objint::get_value(needle)) {
        Some(idx) => Ok(vm.ctx.new_int(idx)),
        None => Err(vm.new_value_error(format!("{} is not in range", needle.borrow()))),
    }
}

fn range_bool(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, Some(vm.ctx.range_type()))]);
    let range = get_value(zelf);
    Ok(vm.new_bool(!range.len().is_zero()))
}

fn get_value<'a>(obj: &'a PyObjectRef) -> Ref<'a, RangeType> {
    Ref::map(obj.borrow(), |x| {
        if let PyObjectPayload::Range { ref range } = x.payload {
            range
        } else {
            panic!("Inner error getting range");
        }
    })
}
