use super::{PyCode, PyStrRef, PyType};
use crate::{
    AsObject, Context, Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    class::PyClassImpl,
    coroutine::Coro,
    frame::FrameRef,
    function::OptionalArg,
    protocol::PyIterReturn,
    types::{IterNext, Iterable, Representable, SelfIter, Unconstructible},
};

#[pyclass(module = false, name = "coroutine")]
#[derive(Debug)]
// PyCoro_Type in CPython
pub struct PyCoroutine {
    inner: Coro,
}

impl PyPayload for PyCoroutine {
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.coroutine_type
    }
}

#[pyclass(with(Py, Unconstructible, IterNext, Representable))]
impl PyCoroutine {
    pub fn as_coro(&self) -> &Coro {
        &self.inner
    }

    pub fn new(frame: FrameRef, name: PyStrRef) -> Self {
        PyCoroutine {
            inner: Coro::new(frame, name),
        }
    }

    #[pygetset(magic)]
    fn name(&self) -> PyStrRef {
        self.inner.name()
    }

    #[pygetset(magic, setter)]
    fn set_name(&self, name: PyStrRef) {
        self.inner.set_name(name)
    }

    #[pymethod(name = "__await__")]
    fn r#await(zelf: PyRef<Self>) -> PyCoroutineWrapper {
        PyCoroutineWrapper { coro: zelf }
    }

    #[pygetset]
    fn cr_await(&self, _vm: &VirtualMachine) -> Option<PyObjectRef> {
        self.inner.frame().yield_from_target()
    }
    #[pygetset]
    fn cr_frame(&self, _vm: &VirtualMachine) -> FrameRef {
        self.inner.frame()
    }
    #[pygetset]
    fn cr_running(&self, _vm: &VirtualMachine) -> bool {
        self.inner.running()
    }
    #[pygetset]
    fn cr_code(&self, _vm: &VirtualMachine) -> PyRef<PyCode> {
        self.inner.frame().code.clone()
    }
    // TODO: coroutine origin tracking:
    // https://docs.python.org/3/library/sys.html#sys.set_coroutine_origin_tracking_depth
    #[pygetset]
    fn cr_origin(&self, _vm: &VirtualMachine) -> Option<(PyStrRef, usize, PyStrRef)> {
        None
    }
}

#[pyclass]
impl Py<PyCoroutine> {
    #[pymethod]
    fn send(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        self.inner.send(self.as_object(), value, vm)
    }

    #[pymethod]
    fn throw(
        &self,
        exc_type: PyObjectRef,
        exc_val: OptionalArg,
        exc_tb: OptionalArg,
        vm: &VirtualMachine,
    ) -> PyResult<PyIterReturn> {
        self.inner.throw(
            self.as_object(),
            exc_type,
            exc_val.unwrap_or_none(vm),
            exc_tb.unwrap_or_none(vm),
            vm,
        )
    }

    #[pymethod]
    fn close(&self, vm: &VirtualMachine) -> PyResult<()> {
        self.inner.close(self.as_object(), vm)
    }
}

impl Unconstructible for PyCoroutine {}

impl Representable for PyCoroutine {
    #[inline]
    fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
        Ok(zelf.inner.repr(zelf.as_object(), zelf.get_id(), vm))
    }
}

impl SelfIter for PyCoroutine {}
impl IterNext for PyCoroutine {
    fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        zelf.send(vm.ctx.none(), vm)
    }
}

#[pyclass(module = false, name = "coroutine_wrapper")]
#[derive(Debug)]
// PyCoroWrapper_Type in CPython
pub struct PyCoroutineWrapper {
    coro: PyRef<PyCoroutine>,
}

impl PyPayload for PyCoroutineWrapper {
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.coroutine_wrapper_type
    }
}

#[pyclass(with(IterNext, Iterable))]
impl PyCoroutineWrapper {
    #[pymethod]
    fn send(&self, val: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        self.coro.send(val, vm)
    }

    #[pymethod]
    fn throw(
        &self,
        exc_type: PyObjectRef,
        exc_val: OptionalArg,
        exc_tb: OptionalArg,
        vm: &VirtualMachine,
    ) -> PyResult<PyIterReturn> {
        self.coro.throw(exc_type, exc_val, exc_tb, vm)
    }
}

impl SelfIter for PyCoroutineWrapper {}
impl IterNext for PyCoroutineWrapper {
    fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        Self::send(zelf, vm.ctx.none(), vm)
    }
}

pub fn init(ctx: &Context) {
    PyCoroutine::extend_class(ctx, ctx.types.coroutine_type);
    PyCoroutineWrapper::extend_class(ctx, ctx.types.coroutine_wrapper_type);
}
