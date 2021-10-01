/*
 * The mythical generator.
 */

use super::{PyCode, PyStrRef, PyTypeRef};
use crate::{
    coroutine::Coro,
    frame::FrameRef,
    function::OptionalArg,
    protocol::PyIterReturn,
    slots::{IteratorIterable, SlotIterator},
    IdProtocol, PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue, VirtualMachine,
};

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

#[pyimpl(with(SlotIterator))]
impl PyGenerator {
    pub fn as_coro(&self) -> &Coro {
        &self.inner
    }

    pub fn new(frame: FrameRef, name: PyStrRef) -> Self {
        PyGenerator {
            inner: Coro::new(frame, name),
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

    #[pyproperty]
    fn gi_frame(&self, _vm: &VirtualMachine) -> FrameRef {
        self.inner.frame()
    }
    #[pyproperty]
    fn gi_running(&self, _vm: &VirtualMachine) -> bool {
        self.inner.running()
    }
    #[pyproperty]
    fn gi_code(&self, _vm: &VirtualMachine) -> PyRef<PyCode> {
        self.inner.frame().code.clone()
    }
    #[pyproperty]
    fn gi_yieldfrom(&self, _vm: &VirtualMachine) -> Option<PyObjectRef> {
        self.inner.frame().yield_from_target()
    }
}

impl IteratorIterable for PyGenerator {}
impl SlotIterator for PyGenerator {
    fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        Self::send(zelf.clone(), vm.ctx.none(), vm)
    }
}

pub fn init(ctx: &PyContext) {
    PyGenerator::extend_class(ctx, &ctx.types.generator_type);
}
