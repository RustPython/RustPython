use super::objcoroinner::Coro;
use super::objtype::PyClassRef;
use crate::frame::FrameRef;
use crate::function::OptionalArg;
use crate::pyobject::{PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

pub type PyCoroutineRef = PyRef<PyCoroutine>;

#[pyclass(name = "coroutine")]
#[derive(Debug)]
pub struct PyCoroutine {
    inner: Coro,
}

impl PyValue for PyCoroutine {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.types.coroutine_type.clone()
    }
}

#[pyimpl]
impl PyCoroutine {
    pub fn as_coro(&self) -> &Coro {
        &self.inner
    }

    pub fn new(frame: FrameRef, vm: &VirtualMachine) -> PyCoroutineRef {
        PyCoroutine {
            inner: Coro::new(frame),
        }
        .into_ref(vm)
    }

    #[pymethod]
    fn send(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.inner.send(value, vm)
    }

    #[pymethod]
    fn throw(
        &self,
        exc_type: PyObjectRef,
        exc_val: OptionalArg,
        exc_tb: OptionalArg,
        vm: &VirtualMachine,
    ) -> PyResult {
        self.inner.throw(
            exc_type,
            exc_val.unwrap_or_else(|| vm.get_none()),
            exc_tb.unwrap_or_else(|| vm.get_none()),
            vm,
        )
    }

    #[pymethod]
    fn close(&self, vm: &VirtualMachine) -> PyResult<()> {
        self.inner.close(vm)
    }

    #[pymethod(name = "__await__")]
    fn r#await(zelf: PyRef<Self>) -> PyCoroutineWrapper {
        PyCoroutineWrapper { coro: zelf }
    }
}

#[pyclass(name = "coroutine_wrapper")]
#[derive(Debug)]
pub struct PyCoroutineWrapper {
    coro: PyCoroutineRef,
}

impl PyValue for PyCoroutineWrapper {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.types.coroutine_wrapper_type.clone()
    }
}

#[pyimpl]
impl PyCoroutineWrapper {
    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>) -> PyRef<Self> {
        zelf
    }

    #[pymethod(name = "__next__")]
    fn next(&self, vm: &VirtualMachine) -> PyResult {
        self.coro.send(vm.get_none(), vm)
    }

    #[pymethod]
    fn send(&self, val: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.coro.send(val, vm)
    }

    #[pymethod]
    fn throw(
        &self,
        exc_type: PyObjectRef,
        exc_val: OptionalArg,
        exc_tb: OptionalArg,
        vm: &VirtualMachine,
    ) -> PyResult {
        self.coro.throw(exc_type, exc_val, exc_tb, vm)
    }
}

pub fn init(ctx: &PyContext) {
    PyCoroutine::extend_class(ctx, &ctx.types.coroutine_type);
    PyCoroutineWrapper::extend_class(ctx, &ctx.types.coroutine_wrapper_type);
}
