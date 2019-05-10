use std::cell::RefCell;
use std::ops::AddAssign;

use num_bigint::BigInt;

use crate::function::OptionalArg;
use crate::obj::objint::{PyInt, PyIntRef};
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{PyClassImpl, PyObjectRef, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

#[pyclass]
#[derive(Debug)]
struct PyItertoolsCount {
    cur: RefCell<BigInt>,
    step: BigInt,
}

impl PyValue for PyItertoolsCount {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("itertools", "count")
    }
}

#[pyimpl]
impl PyItertoolsCount {
    #[pymethod(name = "__new__")]
    fn new(
        _cls: PyClassRef,
        start: OptionalArg<PyIntRef>,
        step: OptionalArg<PyIntRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let start = match start.into_option() {
            Some(int) => int.as_bigint().clone(),
            None => BigInt::from(0),
        };
        let step = match step.into_option() {
            Some(int) => int.as_bigint().clone(),
            None => BigInt::from(1),
        };

        Ok(PyItertoolsCount {
            cur: RefCell::new(start),
            step: step,
        }
        .into_ref(vm)
        .into_object())
    }

    #[pymethod(name = "__next__")]
    fn next(&self, _vm: &VirtualMachine) -> PyResult<PyInt> {
        let result = self.cur.borrow().clone();
        AddAssign::add_assign(&mut self.cur.borrow_mut() as &mut BigInt, &self.step);
        Ok(PyInt::new(result))
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyRef<Self> {
        zelf
    }
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    let count = ctx.new_class("count", ctx.object());
    PyItertoolsCount::extend_class(ctx, &count);

    py_module!(vm, "itertools", {
        "count" => count,
    })
}
