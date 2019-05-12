use std::cell::RefCell;
use std::cmp::Ordering;
use std::ops::{AddAssign, SubAssign};

use num_bigint::BigInt;

use crate::function::OptionalArg;
use crate::obj::objint::{PyInt, PyIntRef};
use crate::obj::objiter::new_stop_iteration;
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

#[pyclass]
#[derive(Debug)]
struct PyItertoolsRepeat {
    object: PyObjectRef,
    times: Option<RefCell<BigInt>>,
}

impl PyValue for PyItertoolsRepeat {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("itertools", "repeat")
    }
}

#[pyimpl]
impl PyItertoolsRepeat {
    #[pymethod(name = "__new__")]
    fn new(
        _cls: PyClassRef,
        object: PyObjectRef,
        times: OptionalArg<PyIntRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let times = match times.into_option() {
            Some(int) => Some(RefCell::new(int.as_bigint().clone())),
            None => None,
        };

        Ok(PyItertoolsRepeat {
            object: object.clone(),
            times: times,
        }
        .into_ref(vm)
        .into_object())
    }

    #[pymethod(name = "__next__")]
    fn next(&self, vm: &VirtualMachine) -> PyResult {
        if self.times.is_some() {
            match self.times.as_ref().unwrap().borrow().cmp(&BigInt::from(0)) {
                Ordering::Less | Ordering::Equal => return Err(new_stop_iteration(vm)),
                _ => (),
            };

            SubAssign::sub_assign(
                &mut self.times.as_ref().unwrap().borrow_mut() as &mut BigInt,
                &BigInt::from(1),
            );
        }

        Ok(self.object.clone())
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

    let repeat = ctx.new_class("repeat", ctx.object());
    PyItertoolsRepeat::extend_class(ctx, &repeat);

    py_module!(vm, "itertools", {
        "count" => count,
        "repeat" => repeat,
    })
}
