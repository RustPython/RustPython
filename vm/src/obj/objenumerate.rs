use std::cell::RefCell;
use std::ops::AddAssign;

use super::objint;
use super::objiter;
use crate::pyobject::{
    PyContext, PyFuncArgs, PyObject, PyObjectRef, PyResult, PyValue, TypeProtocol,
};
use crate::vm::VirtualMachine;
use num_bigint::BigInt;
use num_traits::Zero;

#[derive(Debug)]
pub struct PyEnumerate {
    counter: RefCell<BigInt>,
    iterator: PyObjectRef,
}

impl PyValue for PyEnumerate {
    fn required_type(ctx: &PyContext) -> PyObjectRef {
        ctx.enumerate_type()
    }
}

fn enumerate_new(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(cls, Some(vm.ctx.type_type())), (iterable, None)],
        optional = [(start, Some(vm.ctx.int_type()))]
    );
    let counter = if let Some(x) = start {
        objint::get_value(x)
    } else {
        BigInt::zero()
    };
    let iterator = objiter::get_iter(vm, iterable)?;
    Ok(PyObject::new(
        PyEnumerate {
            counter: RefCell::new(counter),
            iterator,
        },
        cls.clone(),
    ))
}

fn enumerate_next(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(enumerate, Some(vm.ctx.enumerate_type()))]
    );

    if let Some(PyEnumerate {
        ref counter,
        ref iterator,
    }) = enumerate.payload()
    {
        let next_obj = objiter::call_next(vm, iterator)?;
        let result = vm
            .ctx
            .new_tuple(vec![vm.ctx.new_int(counter.borrow().clone()), next_obj]);

        AddAssign::add_assign(&mut counter.borrow_mut() as &mut BigInt, 1);

        Ok(result)
    } else {
        panic!("enumerate doesn't have correct payload");
    }
}

pub fn init(context: &PyContext) {
    let enumerate_type = &context.enumerate_type;
    objiter::iter_type_init(context, enumerate_type);
    context.set_attr(
        enumerate_type,
        "__new__",
        context.new_rustfunc(enumerate_new),
    );
    context.set_attr(
        enumerate_type,
        "__next__",
        context.new_rustfunc(enumerate_next),
    );
}
