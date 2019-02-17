use super::super::pyobject::{
    PyContext, PyFuncArgs, PyObject, PyObjectPayload, PyResult, TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objint;
use super::objiter;
use super::objtype; // Required for arg_check! to use isinstance
use num_bigint::BigInt;
use num_traits::Zero;
use std::ops::AddAssign;

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
        PyObjectPayload::EnumerateIterator { counter, iterator },
        cls.clone(),
    ))
}

fn enumerate_next(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(enumerate, Some(vm.ctx.enumerate_type()))]
    );

    if let PyObjectPayload::EnumerateIterator {
        ref mut counter,
        ref mut iterator,
    } = enumerate.borrow_mut().payload
    {
        let next_obj = objiter::call_next(vm, iterator)?;
        let result = vm
            .ctx
            .new_tuple(vec![vm.ctx.new_int(counter.clone()), next_obj]);

        AddAssign::add_assign(counter, 1);

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
