/*
 * The mythical generator.
 */

use super::objcoroinner::Coro;
use super::objtype::PyClassRef;
use crate::frame::FrameRef;
use crate::function::OptionalArg;
use crate::pyobject::{PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

pub type PyGeneratorRef = PyRef<PyGenerator>;

#[pyclass(name = "generator")]
#[derive(Debug)]
pub struct PyGenerator {
    inner: Coro,
}

impl PyValue for PyGenerator {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.generator_type()
    }
}

#[pyimpl]
impl PyGenerator {
    pub fn as_coro(&self) -> &Coro {
        &self.inner
    }

    pub fn new(frame: FrameRef, vm: &VirtualMachine) -> PyGeneratorRef {
        PyGenerator {
            inner: Coro::new(frame),
        }
        .into_ref(vm)
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyGeneratorRef) -> PyGeneratorRef {
        zelf
    }

    #[pymethod(name = "__next__")]
    fn next(&self, vm: &VirtualMachine) -> PyResult {
        self.send(vm.get_none(), vm)
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
}

pub fn init(ctx: &PyContext) {
    PyGenerator::extend_class(ctx, &ctx.types.generator_type);
}
