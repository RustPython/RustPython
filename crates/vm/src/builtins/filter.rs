use super::{PyType, PyTypeRef};
use crate::{
    Context, Py, PyObjectRef, PyPayload, PyResult, VirtualMachine,
    class::PyClassImpl,
    protocol::{PyIter, PyIterReturn},
    raise_if_stop,
    types::{Constructor, IterNext, Iterable, SelfIter},
};

#[pyclass(module = false, name = "filter", traverse)]
#[derive(Debug)]
pub struct PyFilter {
    predicate: PyObjectRef,
    iterator: PyIter,
}

impl PyPayload for PyFilter {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.filter_type
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

#[pyclass(with(IterNext, Iterable, Constructor), flags(BASETYPE))]
impl PyFilter {
    #[pymethod]
    fn __reduce__(&self, vm: &VirtualMachine) -> (PyTypeRef, (PyObjectRef, PyIter)) {
        (
            vm.ctx.types.filter_type.to_owned(),
            (self.predicate.clone(), self.iterator.clone()),
        )
    }
}

impl SelfIter for PyFilter {}

impl IterNext for PyFilter {
    fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        let predicate = &zelf.predicate;
        loop {
            let next_obj = raise_if_stop!(zelf.iterator.next(vm)?);
            let predicate_value = if vm.is_none(predicate) {
                next_obj.clone()
            } else {
                // the predicate itself can raise StopIteration which does stop the filter iteration
                raise_if_stop!(PyIterReturn::from_pyresult(
                    predicate.call((next_obj.clone(),), vm),
                    vm
                )?)
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
