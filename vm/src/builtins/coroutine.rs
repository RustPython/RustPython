use super::{PyCode, PyStrRef, PyType};
use crate::{
    class::PyClassImpl,
    coroutine::Coro,
    frame::FrameRef,
    function::OptionalArg,
    protocol::PyIterReturn,
    types::{Constructor, IterNext, IterNextIterable, Unconstructible},
    AsObject, Context, Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
};

#[pyclass(module = false, name = "coroutine")]
#[derive(Debug)]
// PyCoro_Type in CPython
pub struct PyCoroutine {
    inner: Coro,
}

impl PyPayload for PyCoroutine {
    fn class(vm: &VirtualMachine) -> &'static Py<PyType> {
        vm.ctx.types.coroutine_type
    }
}

#[pyclass(with(Constructor, IterNext))]
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

    #[pymethod(magic)]
    fn repr(zelf: PyRef<Self>, vm: &VirtualMachine) -> String {
        zelf.inner.repr(zelf.as_object(), zelf.get_id(), vm)
    }

    #[pymethod]
    fn send(zelf: PyRef<Self>, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        zelf.inner.send(zelf.as_object(), value, vm)
    }

    #[pymethod]
    fn throw(
        zelf: PyRef<Self>,
        exc_type: PyObjectRef,
        exc_val: OptionalArg,
        exc_tb: OptionalArg,
        vm: &VirtualMachine,
    ) -> PyResult<PyIterReturn> {
        zelf.inner.throw(
            zelf.as_object(),
            exc_type,
            exc_val.unwrap_or_none(vm),
            exc_tb.unwrap_or_none(vm),
            vm,
        )
    }

    #[pymethod]
    fn close(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<()> {
        zelf.inner.close(zelf.as_object(), vm)
    }

    #[pymethod(name = "__await__")]
    fn r#await(zelf: PyRef<Self>) -> PyCoroutineWrapper {
        PyCoroutineWrapper { coro: zelf }
    }

    #[pygetset]
    fn cr_await(&self, _vm: &VirtualMachine) -> Option<PyObjectRef> {
        self.inner.frame().and_then(|x| x.yield_from_target())
    }
    #[pygetset]
    fn cr_frame(&self, _vm: &VirtualMachine) -> Option<FrameRef> {
        self.inner.frame()
    }
    #[pygetset]
    fn cr_running(&self, _vm: &VirtualMachine) -> bool {
        self.inner.running()
    }
    #[pygetset]
    fn cr_code(&self, _vm: &VirtualMachine) -> Option<PyRef<PyCode>> {
        self.inner.frame().map(|x| x.code.clone())
    }
    // TODO: coroutine origin tracking:
    // https://docs.python.org/3/library/sys.html#sys.set_coroutine_origin_tracking_depth
    #[pygetset]
    fn cr_origin(&self, _vm: &VirtualMachine) -> Option<(PyStrRef, usize, PyStrRef)> {
        None
    }
}
impl Unconstructible for PyCoroutine {}

impl IterNextIterable for PyCoroutine {}
impl IterNext for PyCoroutine {
    fn next(zelf: &crate::Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        Self::send(zelf.to_owned(), vm.ctx.none(), vm)
    }
}

#[pyclass(module = false, name = "coroutine_wrapper")]
#[derive(Debug)]
// PyCoroWrapper_Type in CPython
pub struct PyCoroutineWrapper {
    coro: PyRef<PyCoroutine>,
}

impl PyPayload for PyCoroutineWrapper {
    fn class(vm: &VirtualMachine) -> &'static Py<PyType> {
        vm.ctx.types.coroutine_wrapper_type
    }
}

#[pyclass(with(IterNext))]
impl PyCoroutineWrapper {
    #[pymethod]
    fn send(zelf: PyRef<Self>, val: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        PyCoroutine::send(zelf.coro.clone(), val, vm)
    }

    #[pymethod]
    fn throw(
        &self,
        exc_type: PyObjectRef,
        exc_val: OptionalArg,
        exc_tb: OptionalArg,
        vm: &VirtualMachine,
    ) -> PyResult<PyIterReturn> {
        PyCoroutine::throw(self.coro.clone(), exc_type, exc_val, exc_tb, vm)
    }
}

impl IterNextIterable for PyCoroutineWrapper {}
impl IterNext for PyCoroutineWrapper {
    fn next(zelf: &crate::Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        Self::send(zelf.to_owned(), vm.ctx.none(), vm)
    }
}

pub fn init(ctx: &Context) {
    PyCoroutine::extend_class(ctx, ctx.types.coroutine_type);
    PyCoroutineWrapper::extend_class(ctx, ctx.types.coroutine_wrapper_type);
}
