use super::objcode::PyCodeRef;
use super::objcoroinner::Coro;
use super::objtype::PyClassRef;
use crate::frame::FrameRef;
use crate::function::OptionalArg;
use crate::pyobject::{
    IntoPyObject, PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue,
};
use crate::vm::VirtualMachine;

use std::cell::RefCell;

#[pyclass(name = "async_generator")]
#[derive(Debug)]
pub struct PyAsyncGen {
    inner: Coro,
}
pub type PyAsyncGenRef = PyRef<PyAsyncGen>;

impl PyValue for PyAsyncGen {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.types.async_generator.clone()
    }
}

#[pyimpl]
impl PyAsyncGen {
    pub fn as_coro(&self) -> &Coro {
        &self.inner
    }

    pub fn new(frame: FrameRef, vm: &VirtualMachine) -> PyAsyncGenRef {
        PyAsyncGen {
            inner: Coro::new_async(frame),
        }
        .into_ref(vm)
    }

    #[pymethod(name = "__aiter__")]
    fn aiter(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyRef<Self> {
        zelf
    }

    #[pymethod(name = "__anext__")]
    fn anext(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyAsyncGenASend {
        Self::asend(zelf, vm.get_none(), vm)
    }

    #[pymethod]
    fn asend(zelf: PyRef<Self>, value: PyObjectRef, _vm: &VirtualMachine) -> PyAsyncGenASend {
        PyAsyncGenASend {
            ag: zelf,
            value: RefCell::new(Some(value)),
        }
    }

    #[pymethod]
    fn athrow(
        zelf: PyRef<Self>,
        exc_type: PyObjectRef,
        exc_val: OptionalArg,
        exc_tb: OptionalArg,
        vm: &VirtualMachine,
    ) -> PyAsyncGenAThrow {
        PyAsyncGenAThrow {
            ag: zelf,
            value: RefCell::new(Some((
                exc_type,
                exc_val.unwrap_or_else(|| vm.get_none()),
                exc_tb.unwrap_or_else(|| vm.get_none()),
            ))),
        }
    }

    #[pymethod]
    fn close(&self, vm: &VirtualMachine) -> PyResult<()> {
        self.inner.close(vm)
    }

    #[pyproperty]
    fn ag_await(&self, _vm: &VirtualMachine) -> Option<PyObjectRef> {
        self.inner.frame().yield_from_target()
    }
    #[pyproperty]
    fn ag_frame(&self, _vm: &VirtualMachine) -> FrameRef {
        self.inner.frame()
    }
    #[pyproperty]
    fn ag_running(&self, _vm: &VirtualMachine) -> bool {
        self.inner.running()
    }
    #[pyproperty]
    fn ag_code(&self, _vm: &VirtualMachine) -> PyCodeRef {
        self.inner.frame().code.clone()
    }
}

#[pyclass(name = "async_generator_wrapped_value")]
#[derive(Debug)]
pub(crate) struct PyAsyncGenWrappedValue(pub PyObjectRef);
impl PyValue for PyAsyncGenWrappedValue {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.types.async_generator_wrapped_value.clone()
    }
}

impl PyAsyncGenWrappedValue {
    fn unbox(val: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        match_class!(match val {
            val @ Self => {
                Err(vm.new_exception(
                    vm.ctx.exceptions.stop_iteration.clone(),
                    vec![val.0.clone()],
                ))
            }
            val => Ok(val),
        })
    }
}

#[pyclass(name = "async_generator_asend")]
#[derive(Debug)]
struct PyAsyncGenASend {
    ag: PyAsyncGenRef,
    value: RefCell<Option<PyObjectRef>>,
}

impl PyValue for PyAsyncGenASend {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.types.async_generator_asend.clone()
    }
}

#[pyimpl]
impl PyAsyncGenASend {
    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyRef<Self> {
        zelf
    }
    #[pymethod(name = "__await__")]
    fn r#await(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyRef<Self> {
        zelf
    }

    #[pymethod(name = "__next__")]
    fn next(&self, vm: &VirtualMachine) -> PyResult {
        self.send(vm.get_none(), vm)
    }

    #[pymethod]
    fn send(&self, val: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let val = if self.ag.inner.started() {
            val
        } else if vm.is_none(&val) {
            self.value.replace(None).unwrap_or_else(|| vm.get_none())
        } else {
            return Err(vm.new_type_error(
                "can't send non-None value to a just-started async generator".to_string(),
            ));
        };
        self.ag
            .inner
            .send(val, vm)
            .and_then(|val| PyAsyncGenWrappedValue::unbox(val, vm))
    }

    #[pymethod]
    fn throw(
        &self,
        exc_type: PyObjectRef,
        exc_val: OptionalArg,
        exc_tb: OptionalArg,
        vm: &VirtualMachine,
    ) -> PyResult {
        self.ag
            .inner
            .throw(
                exc_type,
                exc_val.unwrap_or_else(|| vm.get_none()),
                exc_tb.unwrap_or_else(|| vm.get_none()),
                vm,
            )
            .and_then(|val| PyAsyncGenWrappedValue::unbox(val, vm))
    }
}

#[pyclass(name = "async_generator_asend")]
#[derive(Debug)]
struct PyAsyncGenAThrow {
    ag: PyAsyncGenRef,
    value: RefCell<Option<(PyObjectRef, PyObjectRef, PyObjectRef)>>,
}

impl PyValue for PyAsyncGenAThrow {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.types.async_generator_athrow.clone()
    }
}

#[pyimpl]
impl PyAsyncGenAThrow {
    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyRef<Self> {
        zelf
    }
    #[pymethod(name = "__await__")]
    fn r#await(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyRef<Self> {
        zelf
    }

    #[pymethod(name = "__next__")]
    fn next(&self, vm: &VirtualMachine) -> PyResult {
        self.send(vm.get_none(), vm)
    }

    #[pymethod]
    fn send(&self, val: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let val = if self.ag.inner.started() {
            val
        } else if vm.is_none(&val) {
            self.value
                .replace(None)
                .map_or_else(|| Ok(vm.get_none()), |val| val.into_pyobject(vm))?
        } else {
            return Err(vm.new_type_error(
                "can't send non-None value to a just-started async generator".to_string(),
            ));
        };
        self.ag
            .inner
            .send(val, vm)
            .and_then(|val| PyAsyncGenWrappedValue::unbox(val, vm))
    }

    #[pymethod]
    fn throw(
        &self,
        exc_type: PyObjectRef,
        exc_val: OptionalArg,
        exc_tb: OptionalArg,
        vm: &VirtualMachine,
    ) -> PyResult {
        self.ag
            .inner
            .throw(
                exc_type,
                exc_val.unwrap_or_else(|| vm.get_none()),
                exc_tb.unwrap_or_else(|| vm.get_none()),
                vm,
            )
            .and_then(|val| PyAsyncGenWrappedValue::unbox(val, vm))
    }
}

pub fn init(ctx: &PyContext) {
    PyAsyncGen::extend_class(ctx, &ctx.types.async_generator);
    PyAsyncGenASend::extend_class(ctx, &ctx.types.async_generator_asend);
    PyAsyncGenAThrow::extend_class(ctx, &ctx.types.async_generator_athrow);
}
