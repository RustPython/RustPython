/*
 * The mythical generator.
 */

use super::code::PyCodeRef;
use super::pytype::PyTypeRef;
use crate::coroutine::{Coro, Variant};
use crate::frame::FrameRef;
use crate::function::OptionalArg;
use crate::pyobject::{PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

#[pyclass(module = false, name = "generator")]
#[derive(Debug)]
pub struct PyGenerator {
    inner: Coro,
}

impl PyValue for PyGenerator {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.generator_type
    }
}

#[pyimpl]
impl PyGenerator {
    pub fn as_coro(&self) -> &Coro {
        &self.inner
    }

    pub fn new(frame: FrameRef, vm: &VirtualMachine) -> PyRef<Self> {
        PyGenerator {
            inner: Coro::new(frame, Variant::Gen),
        }
        .into_ref(vm)
    }

    // TODO: fix function names situation
    #[pyproperty(magic)]
    fn name(&self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.none()
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>) -> PyRef<Self> {
        zelf
    }

    #[pymethod(name = "__next__")]
    fn next(&self, vm: &VirtualMachine) -> PyResult {
        self.send(vm.ctx.none(), vm)
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

    #[pyproperty]
    fn gi_frame(&self, _vm: &VirtualMachine) -> FrameRef {
        self.inner.frame()
    }
    #[pyproperty]
    fn gi_running(&self, _vm: &VirtualMachine) -> bool {
        self.inner.running()
    }
    #[pyproperty]
    fn gi_code(&self, _vm: &VirtualMachine) -> PyCodeRef {
        self.inner.frame().code.clone()
    }
    #[pyproperty]
    fn gi_yieldfrom(&self, _vm: &VirtualMachine) -> Option<PyObjectRef> {
        self.inner.frame().yield_from_target()
    }
}

pub fn init(ctx: &PyContext) {
    PyGenerator::extend_class(ctx, &ctx.types.generator_type);
}
