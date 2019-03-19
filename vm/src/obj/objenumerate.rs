use std::cell::RefCell;
use std::ops::AddAssign;

use num_bigint::BigInt;
use num_traits::Zero;

use crate::function::{OptionalArg, PyFuncArgs};
use crate::pyobject::{PyContext, PyObjectRef, PyRef, PyResult, PyValue, TypeProtocol};
use crate::vm::VirtualMachine;

use super::objint::PyIntRef;
use super::objiter;
use super::objtype::PyClassRef;

#[derive(Debug)]
pub struct PyEnumerate {
    counter: RefCell<BigInt>,
    iterator: PyObjectRef,
}
type PyEnumerateRef = PyRef<PyEnumerate>;

impl PyValue for PyEnumerate {
    fn class(vm: &mut VirtualMachine) -> PyObjectRef {
        vm.ctx.enumerate_type()
    }
}

fn enumerate_new(
    cls: PyClassRef,
    iterable: PyObjectRef,
    start: OptionalArg<PyIntRef>,
    vm: &mut VirtualMachine,
) -> PyResult<PyEnumerateRef> {
    let counter = match start {
        OptionalArg::Present(start) => start.value.clone(),
        OptionalArg::Missing => BigInt::zero(),
    };

    let iterator = objiter::get_iter(vm, &iterable)?;
    PyEnumerate {
        counter: RefCell::new(counter.clone()),
        iterator,
    }
    .into_ref_with_type(vm, cls)
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
