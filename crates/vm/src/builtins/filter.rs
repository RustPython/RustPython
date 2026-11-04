use super::{PyType, PyTypeRef};
use crate::{
    AsObject, Context, Py, PyObjectRef, PyPayload, PyResult, VirtualMachine,
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

#[derive(FromArgs)]
pub struct FilterArgs {
    #[pyarg(positional)]
    function: PyObjectRef,
    #[pyarg(positional)]
    iterable: PyObjectRef,
}

impl Constructor for PyFilter {
    type Args = FilterArgs;
    const DROP_KWARGS_WHEN_INIT_OVERRIDDEN: bool = true;

    fn py_new(
        _cls: &Py<PyType>,
        Self::Args { function, iterable }: Self::Args,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        let result = iterable.get_iter(vm).map(|iterator| Self {
            predicate: function.clone(),
            iterator,
        });
        drop(iterable);
        drop(function);
        result
    }
}

#[pyclass(with(IterNext, Iterable, Constructor), flags(BASETYPE))]
impl Py<PyFilter> {
    #[pymethod]
    fn __reduce__(&self) -> (PyTypeRef, (PyObjectRef, PyIter)) {
        (
            self.class().to_owned(),
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
            let is_true = if vm.is_none(predicate) || predicate.is(vm.ctx.types.bool_type) {
                next_obj.try_to_bool(vm)?
            } else {
                // the predicate itself can raise StopIteration which does stop the filter iteration
                let predicate_value = raise_if_stop!(PyIterReturn::from_pyresult(
                    predicate.call((next_obj.clone(),), vm),
                    vm
                )?);
                predicate_value.try_to_bool(vm)?
            };

            if is_true {
                return Ok(PyIterReturn::Return(next_obj));
            }
        }
    }
}

pub(crate) fn init(context: &'static Context) {
    PyFilter::extend_class(context, context.types.filter_type);
}
