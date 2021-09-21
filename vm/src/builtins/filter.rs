use super::pytype::PyTypeRef;
use crate::iterator;
use crate::slots::{PyIter, SlotConstructor};
use crate::vm::VirtualMachine;
use crate::{PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue};

/// filter(function or None, iterable) --> filter object
///
/// Return an iterator yielding those items of iterable for which function(item)
/// is true. If function is None, return the items that are true.
#[pyclass(module = false, name = "filter")]
#[derive(Debug)]
pub struct PyFilter {
    predicate: PyObjectRef,
    iterator: PyObjectRef,
}

impl PyValue for PyFilter {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.filter_type
    }
}

#[derive(FromArgs)]
pub struct FilterArgs {
    #[pyarg(positional)]
    function: PyObjectRef,
    #[pyarg(positional)]
    iterable: PyObjectRef,
}

impl SlotConstructor for PyFilter {
    type Args = FilterArgs;

    fn py_new(
        cls: PyTypeRef,
        Self::Args { function, iterable }: Self::Args,
        vm: &VirtualMachine,
    ) -> PyResult {
        let iterator = iterator::get_iter(vm, iterable)?;

        Self {
            predicate: function,
            iterator,
        }
        .into_pyresult_with_type(vm, cls)
    }
}

#[pyimpl(with(PyIter, SlotConstructor), flags(BASETYPE))]
impl PyFilter {}

impl PyIter for PyFilter {
    fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        let predicate = &zelf.predicate;
        let iterator = &zelf.iterator;
        loop {
            let next_obj = iterator::call_next(vm, iterator)?;
            let predicate_value = if vm.is_none(predicate) {
                next_obj.clone()
            } else {
                // the predicate itself can raise StopIteration which does stop the filter
                // iteration
                vm.invoke(predicate, vec![next_obj.clone()])?
            };
            if predicate_value.try_to_bool(vm)? {
                return Ok(next_obj);
            }
        }
    }
}

pub fn init(context: &PyContext) {
    PyFilter::extend_class(context, &context.types.filter_type);
}
