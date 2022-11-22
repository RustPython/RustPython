/*
 * The mythical generator.
 */

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

#[pyclass(module = false, name = "generator")]
#[derive(Debug)]
pub struct PyGenerator {
    inner: Coro,
}

impl PyPayload for PyGenerator {
    fn class(vm: &VirtualMachine) -> &'static Py<PyType> {
        vm.ctx.types.generator_type
    }
}

#[pyclass(with(Constructor, IterNext))]
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

    #[pygetset]
    fn gi_frame(&self, _vm: &VirtualMachine) -> Option<FrameRef> {
        self.inner.frame()
    }
    #[pygetset]
    fn gi_running(&self, _vm: &VirtualMachine) -> bool {
        self.inner.running()
    }
    #[pygetset]
    fn gi_code(&self, _vm: &VirtualMachine) -> Option<PyRef<PyCode>> {
        self.inner.frame().map(|x| x.code.clone())
    }
    #[pygetset]
    fn gi_yieldfrom(&self, _vm: &VirtualMachine) -> Option<PyObjectRef> {
        self.inner.frame().and_then(|x| x.yield_from_target())
    }
}
impl Unconstructible for PyGenerator {}

impl IterNextIterable for PyGenerator {}
impl IterNext for PyGenerator {
    fn next(zelf: &crate::Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        Self::send(zelf.to_owned(), vm.ctx.none(), vm)
    }
}

pub fn init(ctx: &Context) {
    PyGenerator::extend_class(ctx, ctx.types.generator_type);
}
