/*
 * The mythical generator.
 */

use super::code::PyCodeRef;
use super::pystr::PyStrRef;
use super::pytype::PyTypeRef;
use crate::coroutine::{Coro, Variant};
use crate::frame::FrameRef;
use crate::function::OptionalArg;
use crate::pyobject::{IdProtocol, PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue};
use crate::slots::PyIter;
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

#[pyimpl(with(PyIter))]
impl PyGenerator {
    pub fn as_coro(&self) -> &Coro {
        &self.inner
    }

    pub fn new(frame: FrameRef, name: PyStrRef) -> Self {
        PyGenerator {
            inner: Coro::new(frame, Variant::Gen, name),
        }
    }

    #[pyproperty(magic)]
    fn name(&self) -> PyStrRef {
        self.inner.name()
    }

    #[pyproperty(magic, setter)]
    fn set_name(&self, name: PyStrRef) {
        self.inner.set_name(name)
    }

    #[pymethod(magic)]
    fn repr(zelf: PyRef<Self>) -> String {
        zelf.inner.repr(zelf.get_id())
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

impl PyIter for PyGenerator {
    fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        zelf.send(vm.ctx.none(), vm)
    }
}

pub fn init(ctx: &PyContext) {
    PyGenerator::extend_class(ctx, &ctx.types.generator_type);
}
