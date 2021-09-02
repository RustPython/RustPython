use super::pytype::PyTypeRef;
use crate::function::Args;
use crate::iterator;
use crate::slots::{PyIter, SlotConstructor};
use crate::vm::VirtualMachine;
use crate::{PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue};

#[pyclass(module = false, name = "zip")]
#[derive(Debug)]
pub struct PyZip {
    iterators: Vec<PyObjectRef>,
}

impl PyValue for PyZip {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.zip_type
    }
}

impl SlotConstructor for PyZip {
    type Args = Args;

    fn py_new(cls: PyTypeRef, iterables: Self::Args, vm: &VirtualMachine) -> PyResult {
        let iterators = iterables
            .into_iter()
            .map(|iterable| iterator::get_iter(vm, iterable))
            .collect::<Result<Vec<_>, _>>()?;
        PyZip { iterators }.into_pyresult_with_type(vm, cls)
    }
}

#[pyimpl(with(PyIter, SlotConstructor), flags(BASETYPE))]
impl PyZip {}

impl PyIter for PyZip {
    fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        if zelf.iterators.is_empty() {
            Err(vm.new_stop_iteration())
        } else {
            let next_objs = zelf
                .iterators
                .iter()
                .map(|iterator| iterator::call_next(vm, iterator))
                .collect::<Result<Vec<_>, _>>()?;

            Ok(vm.ctx.new_tuple(next_objs))
        }
    }
}

pub fn init(context: &PyContext) {
    PyZip::extend_class(context, &context.types.zip_type);
}
