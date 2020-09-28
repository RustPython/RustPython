use super::code::PyCodeRef;
use super::pystr::PyStrRef;
use super::pytype::PyTypeRef;
use crate::coroutine::{Coro, Variant};
use crate::frame::FrameRef;
use crate::function::OptionalArg;
use crate::pyobject::{PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

type PyCoroutineRef = PyRef<PyCoroutine>;

#[pyclass(module = false, name = "coroutine")]
#[derive(Debug)]
pub struct PyCoroutine {
    inner: Coro,
}

impl PyValue for PyCoroutine {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.coroutine_type
    }
}

#[pyimpl]
impl PyCoroutine {
    pub fn as_coro(&self) -> &Coro {
        &self.inner
    }

    pub fn new(frame: FrameRef, vm: &VirtualMachine) -> PyRef<Self> {
        PyCoroutine {
            inner: Coro::new(frame, Variant::Coroutine),
        }
        .into_ref(vm)
    }

    // TODO: fix function names situation
    #[pyproperty(magic)]
    fn name(&self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.none()
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
            exc_val.unwrap_or_none(vm),
            exc_tb.unwrap_or_none(vm),
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

    #[pyproperty]
    fn cr_await(&self, _vm: &VirtualMachine) -> Option<PyObjectRef> {
        self.inner.frame().yield_from_target()
    }
    #[pyproperty]
    fn cr_frame(&self, _vm: &VirtualMachine) -> FrameRef {
        self.inner.frame()
    }
    #[pyproperty]
    fn cr_running(&self, _vm: &VirtualMachine) -> bool {
        self.inner.running()
    }
    #[pyproperty]
    fn cr_code(&self, _vm: &VirtualMachine) -> PyCodeRef {
        self.inner.frame().code.clone()
    }
    // TODO: coroutine origin tracking:
    // https://docs.python.org/3/library/sys.html#sys.set_coroutine_origin_tracking_depth
    #[pyproperty]
    fn cr_origin(&self, _vm: &VirtualMachine) -> Option<(PyStrRef, usize, PyStrRef)> {
        None
    }
}

#[pyclass(module = false, name = "coroutine_wrapper")]
#[derive(Debug)]
pub struct PyCoroutineWrapper {
    coro: PyCoroutineRef,
}

impl PyValue for PyCoroutineWrapper {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.coroutine_wrapper_type
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
        self.coro.send(vm.ctx.none(), vm)
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
