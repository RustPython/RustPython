/*
 * The mythical generator.
 */

use super::{PyCode, PyGenericAlias, PyStrRef, PyType, PyTypeRef};
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
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.generator_type
    }
}

#[pyclass(with(Py, Unconstructible, IterNext, Iterable))]
impl PyGenerator {
    pub const fn as_coro(&self) -> &Coro {
        &self.inner
    }

    pub fn new(frame: FrameRef, name: PyStrRef) -> Self {
        Self {
            inner: Coro::new(frame, name),
        }
    }

    #[pygetset]
    fn __name__(&self) -> PyStrRef {
        self.inner.name()
    }

    #[pygetset(setter)]
    fn set___name__(&self, name: PyStrRef) {
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

    #[pyclassmethod]
    fn __class_getitem__(cls: PyTypeRef, args: PyObjectRef, vm: &VirtualMachine) -> PyGenericAlias {
        PyGenericAlias::from_args(cls, args, vm)
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
