use super::super::pyobject::{
    PyContext, PyFuncArgs, PyObject, PyObjectPayload, PyObjectRef, PyResult, TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objint;
use super::objtype;
use num_bigint::ToBigInt;
use num_traits::ToPrimitive;

#[derive(Debug, Copy, Clone)]
pub struct RangeType {
    // Unfortunately Rust's built in range type doesn't support things like indexing
    // or ranges where start > end so we need to roll our own.
    pub start: i64,
    pub end: i64,
    pub step: i64,
}

impl RangeType {
    #[inline]
    pub fn len(&self) -> usize {
        if self.is_empty() {
            0usize
        } else {
            ((self.end - self.start) / self.step).abs() as usize
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        (self.start <= self.end && self.step < 0) || (self.start >= self.end && self.step > 0)
    }

    #[inline]
    pub fn forward(&self) -> bool {
        self.start < self.end
    }

    #[inline]
    pub fn get(&self, index: i64) -> Option<i64> {
        let result = self.start + self.step * index;

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
    context.set_attr(&range_type, "__iter__", context.new_rustfunc(range_iter));
    context.set_attr(&range_type, "__len__", context.new_rustfunc(range_len));
    context.set_attr(
        &range_type,
        "__getitem__",
        context.new_rustfunc(range_getitem),
    );
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
        objint::get_value(first).to_i64().unwrap()
    } else {
        0i64
    };

    let end = if let Some(pyint) = second {
        objint::get_value(pyint).to_i64().unwrap()
    } else {
        objint::get_value(first).to_i64().unwrap()
    };

    let step = if let Some(pyint) = step {
        objint::get_value(pyint).to_i64().unwrap()
    } else {
        1i64
    };

    if step == 0 {
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

    let len = match zelf.borrow().payload {
        PyObjectPayload::Range { ref range } => range.len(),
        _ => unreachable!(),
    };

    Ok(vm.ctx.new_int(len.to_bigint().unwrap()))
}

fn range_getitem(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.range_type())), (subscript, None)]
    );
    let zrange = if let PyObjectPayload::Range { range } = zelf.borrow().payload {
        range.clone()
    } else {
        unreachable!()
    };

    match subscript.borrow().payload {
        PyObjectPayload::Integer { ref value } => {
            if let Some(int) = zrange.get(value.to_i64().unwrap()) {
                Ok(PyObject::new(
                    PyObjectPayload::Integer {
                        value: int.to_bigint().unwrap(),
                    },
                    vm.ctx.int_type(),
                ))
            } else {
                Err(vm.new_index_error("range object index out of range".to_string()))
            }
        }
        PyObjectPayload::Slice { start, stop, step } => {
            let new_start = if let Some(int) = start {
                if let Some(i) = zrange.get(int.into()) {
                    i as i64
                } else {
                    zrange.start
                }
            } else {
                zrange.start
            };

            let new_end = if let Some(int) = stop {
                if let Some(i) = zrange.get(int.into()) {
                    i as i64
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
