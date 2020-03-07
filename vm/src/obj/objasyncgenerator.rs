use super::objcode::PyCodeRef;
use super::objcoroinner::{Coro, Variant};
use super::objtype::{self, PyClassRef};
use crate::frame::FrameRef;
use crate::function::OptionalArg;
use crate::pyobject::{PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

use std::cell::Cell;

#[pyclass(name = "async_generator")]
#[derive(Debug)]
pub struct PyAsyncGen {
    inner: Coro,
    running_async: Cell<bool>,
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
            inner: Coro::new(frame, Variant::AsyncGen),
            running_async: Cell::new(false),
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
            state: Cell::new(AwaitableState::Init),
            value,
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
            aclose: false,
            state: Cell::new(AwaitableState::Init),
            value: (
                exc_type,
                exc_val.unwrap_or_else(|| vm.get_none()),
                exc_tb.unwrap_or_else(|| vm.get_none()),
            ),
        }
    }

    #[pymethod]
    fn aclose(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyAsyncGenAThrow {
        PyAsyncGenAThrow {
            ag: zelf,
            aclose: true,
            state: Cell::new(AwaitableState::Init),
            value: (
                vm.ctx.exceptions.generator_exit.clone().into_object(),
                vm.get_none(),
                vm.get_none(),
            ),
        }
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
    fn unbox(ag: &PyAsyncGen, val: PyResult, vm: &VirtualMachine) -> PyResult {
        if let Err(ref e) = val {
            if objtype::isinstance(&e, &vm.ctx.exceptions.stop_async_iteration)
                || objtype::isinstance(&e, &vm.ctx.exceptions.generator_exit)
            {
                ag.inner.closed.set(true);
            }
            ag.running_async.set(false);
        }
        let val = val?;

        match_class!(match val {
            val @ Self => {
                ag.running_async.set(false);
                Err(vm.new_exception(
                    vm.ctx.exceptions.stop_iteration.clone(),
                    vec![val.0.clone()],
                ))
            }
            val => Ok(val),
        })
    }
}

#[derive(Debug, Clone, Copy)]
enum AwaitableState {
    Init,
    Iter,
    Closed,
}

#[pyclass(name = "async_generator_asend")]
#[derive(Debug)]
struct PyAsyncGenASend {
    ag: PyAsyncGenRef,
    state: Cell<AwaitableState>,
    value: PyObjectRef,
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
        let val = match self.state.get() {
            AwaitableState::Closed => {
                return Err(vm.new_runtime_error(
                    "cannot reuse already awaited __anext__()/asend()".to_owned(),
                ))
            }
            AwaitableState::Iter => val, // already running, all good
            AwaitableState::Init => {
                if self.ag.running_async.get() {
                    return Err(vm.new_runtime_error(
                        "anext(): asynchronous generator is already running".to_owned(),
                    ));
                }
                self.ag.running_async.set(true);
                self.state.set(AwaitableState::Iter);
                if vm.is_none(&val) {
                    self.value.clone()
                } else {
                    val
                }
            }
        };
        let res = self.ag.inner.send(val, vm);
        let res = PyAsyncGenWrappedValue::unbox(&self.ag, res, vm);
        if res.is_err() {
            self.state.set(AwaitableState::Closed);
        }
        res
    }

    #[pymethod]
    fn throw(
        &self,
        exc_type: PyObjectRef,
        exc_val: OptionalArg,
        exc_tb: OptionalArg,
        vm: &VirtualMachine,
    ) -> PyResult {
        if let AwaitableState::Closed = self.state.get() {
            return Err(
                vm.new_runtime_error("cannot reuse already awaited __anext__()/asend()".to_owned())
            );
        }

        let res = self.ag.inner.throw(
            exc_type,
            exc_val.unwrap_or_else(|| vm.get_none()),
            exc_tb.unwrap_or_else(|| vm.get_none()),
            vm,
        );
        let res = PyAsyncGenWrappedValue::unbox(&self.ag, res, vm);
        if res.is_err() {
            self.state.set(AwaitableState::Closed);
        }
        res
    }

    #[pymethod]
    fn close(&self) {
        self.state.set(AwaitableState::Closed);
    }
}

#[pyclass(name = "async_generator_athrow")]
#[derive(Debug)]
struct PyAsyncGenAThrow {
    ag: PyAsyncGenRef,
    aclose: bool,
    state: Cell<AwaitableState>,
    value: (PyObjectRef, PyObjectRef, PyObjectRef),
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
        match self.state.get() {
            AwaitableState::Closed => {
                Err(vm
                    .new_runtime_error("cannot reuse already awaited aclose()/athrow()".to_owned()))
            }
            AwaitableState::Init => {
                if self.ag.running_async.get() {
                    self.state.set(AwaitableState::Closed);
                    let msg = if self.aclose {
                        "aclose(): asynchronous generator is already running"
                    } else {
                        "athrow(): asynchronous generator is already running"
                    };
                    return Err(vm.new_runtime_error(msg.to_owned()));
                }
                if self.ag.inner.closed() {
                    self.state.set(AwaitableState::Closed);
                    return Err(vm.new_exception_empty(vm.ctx.exceptions.stop_iteration.clone()));
                }
                if !vm.is_none(&val) {
                    return Err(vm.new_runtime_error(
                        "can't send non-None value to a just-started coroutine".to_owned(),
                    ));
                }
                self.state.set(AwaitableState::Iter);
                self.ag.running_async.set(true);

                let (ty, val, tb) = self.value.clone();
                let ret = self.ag.inner.throw(ty, val, tb, vm);
                let ret = if self.aclose {
                    self.check_no_ignore(&ret, vm)?;
                    ret
                } else {
                    PyAsyncGenWrappedValue::unbox(&self.ag, ret, vm)
                };
                self.check_error(ret, vm)
            }
            AwaitableState::Iter => {
                let ret = self.ag.inner.send(val, vm);
                if self.aclose {
                    self.check_no_ignore(&ret, vm)?;
                    self.check_error(ret, vm)
                } else {
                    PyAsyncGenWrappedValue::unbox(&self.ag, ret, vm)
                }
            }
        }
    }

    #[pymethod]
    fn throw(
        &self,
        exc_type: PyObjectRef,
        exc_val: OptionalArg,
        exc_tb: OptionalArg,
        vm: &VirtualMachine,
    ) -> PyResult {
        let ret = self.ag.inner.throw(
            exc_type,
            exc_val.unwrap_or_else(|| vm.get_none()),
            exc_tb.unwrap_or_else(|| vm.get_none()),
            vm,
        );
        if self.aclose {
            self.check_no_ignore(&ret, vm)?;
            self.check_error(ret, vm)
        } else {
            PyAsyncGenWrappedValue::unbox(&self.ag, ret, vm)
        }
    }

    #[pymethod]
    fn close(&self) {
        self.state.set(AwaitableState::Closed);
    }

    fn check_no_ignore(&self, res: &PyResult, vm: &VirtualMachine) -> PyResult<()> {
        if let Ok(ref v) = res {
            if v.payload_is::<PyAsyncGenWrappedValue>() {
                return Err(
                    vm.new_runtime_error("async generator ignored GeneratorExit".to_owned())
                );
            }
        }
        Ok(())
    }
    fn check_error(&self, res: PyResult, vm: &VirtualMachine) -> PyResult {
        if self.aclose {
            if let Err(ref e) = res {
                if objtype::isinstance(&e, &vm.ctx.exceptions.stop_async_iteration)
                    || objtype::isinstance(&e, &vm.ctx.exceptions.generator_exit)
                {
                    return Err(vm.new_exception_empty(vm.ctx.exceptions.stop_iteration.clone()));
                }
            }
        }
        res
    }
}

pub fn init(ctx: &PyContext) {
    PyAsyncGen::extend_class(ctx, &ctx.types.async_generator);
    PyAsyncGenASend::extend_class(ctx, &ctx.types.async_generator_asend);
    PyAsyncGenAThrow::extend_class(ctx, &ctx.types.async_generator_athrow);
}
