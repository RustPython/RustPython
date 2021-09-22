use super::PyTypeRef;
use crate::{
    function::PosArgs,
    protocol::PyIter,
    slots::{IteratorIterable, SlotConstructor, SlotIterator},
    PyClassImpl, PyContext, PyRef, PyResult, PyValue, VirtualMachine,
};

#[pyclass(module = false, name = "zip")]
#[derive(Debug)]
pub struct PyZip {
    iterators: Vec<PyIter>,
}

impl PyValue for PyZip {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.zip_type
    }
}

impl SlotConstructor for PyZip {
    type Args = PosArgs<PyIter>;

    fn py_new(cls: PyTypeRef, iterators: Self::Args, vm: &VirtualMachine) -> PyResult {
        let iterators = iterators.into_vec();
        PyZip { iterators }.into_pyresult_with_type(vm, cls)
    }
}

#[pyimpl(with(SlotIterator, SlotConstructor), flags(BASETYPE))]
impl PyZip {}

impl IteratorIterable for PyZip {}
impl SlotIterator for PyZip {
    fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        if zelf.iterators.is_empty() {
            Err(vm.new_stop_iteration())
        } else {
            let next_objs = zelf
                .iterators
                .iter()
                .map(|iterator| iterator.next(vm))
                .collect::<Result<Vec<_>, _>>()?;

            Ok(vm.ctx.new_tuple(next_objs))
        }
    }
}

pub fn init(context: &PyContext) {
    PyZip::extend_class(context, &context.types.zip_type);
}
