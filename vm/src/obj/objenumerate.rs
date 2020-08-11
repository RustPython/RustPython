use crate::common::cell::PyRwLock;
use std::ops::AddAssign;

use num_bigint::BigInt;
use num_traits::Zero;

use super::objint::PyIntRef;
use super::objiter;
use super::objtype::PyClassRef;
use crate::function::OptionalArg;
use crate::pyobject::{BorrowValue, PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

#[pyclass]
#[derive(Debug)]
pub struct PyEnumerate {
    counter: PyRwLock<BigInt>,
    iterator: PyObjectRef,
}
type PyEnumerateRef = PyRef<PyEnumerate>;

impl PyValue for PyEnumerate {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.enumerate_type()
    }
}

#[pyimpl]
impl PyEnumerate {
    #[pyslot]
    fn tp_new(
        cls: PyClassRef,
        iterable: PyObjectRef,
        start: OptionalArg<PyIntRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyEnumerateRef> {
        let counter = match start {
            OptionalArg::Present(start) => start.borrow_value().clone(),
            OptionalArg::Missing => BigInt::zero(),
        };

        let iterator = objiter::get_iter(vm, &iterable)?;
        PyEnumerate {
            counter: PyRwLock::new(counter),
            iterator,
        }
        .into_ref_with_type(vm, cls)
    }

    #[pymethod(name = "__next__")]
    fn next(&self, vm: &VirtualMachine) -> PyResult<(BigInt, PyObjectRef)> {
        let next_obj = objiter::call_next(vm, &self.iterator)?;
        let mut counter = self.counter.write();
        let position = counter.clone();
        AddAssign::add_assign(&mut counter as &mut BigInt, 1);
        Ok((position, next_obj))
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>) -> PyRef<Self> {
        zelf
    }
}

pub fn init(context: &PyContext) {
    PyEnumerate::extend_class(context, &context.types.enumerate_type);
}
