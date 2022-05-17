use super::PyTypeRef;
use crate::{
    class::PyClassImpl,
    protocol::{PyIter, PyIterReturn},
    types::{Constructor, IterNext, IterNextIterable},
    Context, PyObjectRef, PyPayload, PyResult, VirtualMachine,
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

impl PyPayload for PyFilter {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        vm.ctx.types.filter_type
    }
}

impl Constructor for PyFilter {
    type Args = (PyObjectRef, PyIter);

    fn py_new(cls: PyTypeRef, (function, iterator): Self::Args, vm: &VirtualMachine) -> PyResult {
        Self {
            predicate: function,
            iterator,
        }
        .into_ref_with_type(vm, cls)
        .map(Into::into)
    }
}

#[pyimpl(with(IterNext, Constructor), flags(BASETYPE))]
impl PyFilter {
    #[pymethod(magic)]
    fn reduce(&self, vm: &VirtualMachine) -> (PyTypeRef, (PyObjectRef, PyIter)) {
        (
            vm.ctx.types.filter_type.clone(),
            (self.predicate.clone(), self.iterator.clone()),
        )
    }
}

impl IterNextIterable for PyFilter {}
impl IterNext for PyFilter {
    fn next(zelf: &crate::Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        let predicate = &zelf.predicate;
        loop {
            let next_obj = match zelf.iterator.next(vm)? {
                PyIterReturn::Return(obj) => obj,
                PyIterReturn::StopIteration(v) => return Ok(PyIterReturn::StopIteration(v)),
            };
            let predicate_value = if vm.is_none(predicate) {
                next_obj.clone()
            } else {
                // the predicate itself can raise StopIteration which does stop the filter
                // iteration
                match PyIterReturn::from_pyresult(vm.invoke(predicate, (next_obj.clone(),)), vm)? {
                    PyIterReturn::Return(obj) => obj,
                    PyIterReturn::StopIteration(v) => return Ok(PyIterReturn::StopIteration(v)),
                }
            };
            if predicate_value.try_to_bool(vm)? {
                return Ok(PyIterReturn::Return(next_obj));
            }
        }
    }
}

pub fn init(context: &Context) {
    PyFilter::extend_class(context, context.types.filter_type);
}
