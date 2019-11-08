use std::cell::RefCell;
use std::ops::AddAssign;

use num_bigint::BigInt;
use num_traits::Zero;

use super::objint::PyIntRef;
use super::objiter;
use super::objtype::PyClassRef;
use crate::function::OptionalArg;
use crate::pyobject::{PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

#[pyclass]
#[derive(Debug)]
pub struct PyEnumerate {
    counter: RefCell<BigInt>,
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
    #[pyslot(new)]
    fn tp_new(
        cls: PyClassRef,
        iterable: PyObjectRef,
        start: OptionalArg<PyIntRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyEnumerateRef> {
        let counter = match start {
            OptionalArg::Present(start) => start.as_bigint().clone(),
            OptionalArg::Missing => BigInt::zero(),
        };

        let iterator = objiter::get_iter(vm, &iterable)?;
        PyEnumerate {
            counter: RefCell::new(counter.clone()),
            iterator,
        }
        .into_ref_with_type(vm, cls)
    }

    #[pymethod(name = "__next__")]
    fn next(&self, vm: &VirtualMachine) -> PyResult {
        let iterator = &self.iterator;
        let counter = &self.counter;
        let next_obj = objiter::call_next(vm, iterator)?;
        let result = vm
            .ctx
            .new_tuple(vec![vm.ctx.new_bigint(&counter.borrow()), next_obj]);

        AddAssign::add_assign(&mut counter.borrow_mut() as &mut BigInt, 1);

        Ok(result)
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyRef<Self> {
        zelf
    }
}

pub fn init(context: &PyContext) {
    PyEnumerate::extend_class(context, &context.types.enumerate_type);
}
