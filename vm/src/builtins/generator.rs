/*
 * The mythical generator.
 */

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

#[pyclass(module = false, name = "generator")]
#[derive(Debug)]
pub struct PyGenerator {
    inner: Coro,
}

impl PyPayload for PyGenerator {
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.generator_type
    }
}

#[pyclass(with(Py, Unconstructible, IterNext, Iterable))]
impl PyGenerator {
    pub fn as_coro(&self) -> &Coro {
        &self.inner
    }

    pub fn new(frame: FrameRef, name: PyStrRef) -> Self {
        PyGenerator {
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

    #[pygetset]
    fn gi_frame(&self, _vm: &VirtualMachine) -> FrameRef {
        self.inner.frame()
    }
    #[pygetset]
    fn gi_running(&self, _vm: &VirtualMachine) -> bool {
        self.inner.running()
    }
    #[pygetset]
    fn gi_code(&self, _vm: &VirtualMachine) -> PyRef<PyCode> {
        self.inner.frame().code.clone()
    }
    #[pygetset]
    fn gi_yieldfrom(&self, _vm: &VirtualMachine) -> Option<PyObjectRef> {
        self.inner.frame().yield_from_target()
    }
}

#[pyclass]
impl Py<PyGenerator> {
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

impl Unconstructible for PyGenerator {}

impl Representable for PyGenerator {
    #[inline]
    fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
        Ok(zelf.inner.repr(zelf.as_object(), zelf.get_id(), vm))
    }
}

impl SelfIter for PyGenerator {}
impl IterNext for PyGenerator {
    fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        zelf.send(vm.ctx.none(), vm)
    }
}

pub fn init(ctx: &Context) {
    PyGenerator::extend_class(ctx, ctx.types.generator_type);
}
