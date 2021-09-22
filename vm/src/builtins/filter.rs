use super::PyTypeRef;
use crate::{
    protocol::PyIter,
    slots::{IteratorIterable, SlotConstructor, SlotIterator},
    PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue, VirtualMachine,
};

/// filter(function or None, iterable) --> filter object
///
/// Return an iterator yielding those items of iterable for which function(item)
/// is true. If function is None, return the items that are true.
#[pyclass(module = false, name = "filter")]
#[derive(Debug)]
pub struct PyFilter {
    predicate: PyObjectRef,
    iterator: PyIter,
}

impl PyValue for PyFilter {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.filter_type
    }
}

impl SlotConstructor for PyFilter {
    type Args = (PyObjectRef, PyIter);

    fn py_new(cls: PyTypeRef, (function, iterator): Self::Args, vm: &VirtualMachine) -> PyResult {
        Self {
            predicate: function,
            iterator,
        }
        .into_pyresult_with_type(vm, cls)
    }
}

#[pyimpl(with(SlotIterator, SlotConstructor), flags(BASETYPE))]
impl PyFilter {}

impl IteratorIterable for PyFilter {}
impl SlotIterator for PyFilter {
    fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        let predicate = &zelf.predicate;
        loop {
            let next_obj = zelf.iterator.next(vm)?;
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
