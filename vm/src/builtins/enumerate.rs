use crate::common::lock::PyRwLock;

use num_bigint::BigInt;
use num_traits::Zero;

use super::int::PyIntRef;
use super::pytype::PyTypeRef;
use crate::function::OptionalArg;
use crate::iterator;
use crate::pyobject::{
    BorrowValue, IntoPyObject, PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue,
};
use crate::slots::PyIter;
use crate::vm::VirtualMachine;

#[pyclass(module = false, name = "enumerate")]
#[derive(Debug)]
pub struct PyEnumerate {
    counter: PyRwLock<BigInt>,
    iterator: PyObjectRef,
}

impl PyValue for PyEnumerate {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.enumerate_type
    }
}

#[derive(FromArgs)]
struct EnumerateArgs {
    #[pyarg(any)]
    iterable: PyObjectRef,
    #[pyarg(any, optional)]
    start: OptionalArg<PyIntRef>,
}

#[pyimpl(with(PyIter))]
impl PyEnumerate {
    #[pyslot]
    fn tp_new(cls: PyTypeRef, args: EnumerateArgs, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        let counter = match args.start {
            OptionalArg::Present(start) => start.borrow_value().clone(),
            OptionalArg::Missing => BigInt::zero(),
        };

        let iterator = iterator::get_iter(vm, args.iterable)?;
        PyEnumerate {
            counter: PyRwLock::new(counter),
            iterator,
        }
        .into_ref_with_type(vm, cls)
    }
}

impl PyIter for PyEnumerate {
    fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        let next_obj = iterator::call_next(vm, &zelf.iterator)?;
        let mut counter = zelf.counter.write();
        let position = counter.clone();
        *counter += 1;
        Ok((position, next_obj).into_pyobject(vm))
    }
}

pub fn init(context: &PyContext) {
    PyEnumerate::extend_class(context, &context.types.enumerate_type);
}
