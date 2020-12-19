use super::code::PyCodeRef;
use super::pystr::PyStrRef;
use super::pytype::PyTypeRef;
use crate::coroutine::{Coro, Variant};
use crate::exceptions::PyBaseExceptionRef;
use crate::frame::FrameRef;
use crate::function::OptionalArg;
use crate::pyobject::{
    IdProtocol, PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue, TypeProtocol,
};
use crate::slots::PyIter;
use crate::vm::VirtualMachine;

use crossbeam_utils::atomic::AtomicCell;

#[pyclass(name = "async_generator", module = false)]
#[derive(Debug)]
pub struct PyAsyncGen {
    inner: Coro,
    running_async: AtomicCell<bool>,
}
type PyAsyncGenRef = PyRef<PyAsyncGen>;

impl PyValue for PyAsyncGen {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.async_generator
    }
}

#[pyimpl]
impl PyAsyncGen {
    pub fn as_coro(&self) -> &Coro {
        &self.inner
    }

    pub fn new(frame: FrameRef, name: PyStrRef) -> Self {
        PyAsyncGen {
            inner: Coro::new(frame, Variant::AsyncGen, name),
            running_async: AtomicCell::new(false),
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

    #[pymethod(name = "__aiter__")]
    fn aiter(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyRef<Self> {
        zelf
    }

    #[pymethod(name = "__anext__")]
    fn anext(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyAsyncGenASend {
        Self::asend(zelf, vm.ctx.none(), vm)
    }

    #[pymethod]
    fn asend(zelf: PyRef<Self>, value: PyObjectRef, _vm: &VirtualMachine) -> PyAsyncGenASend {
        PyAsyncGenASend {
            ag: zelf,
            state: AtomicCell::new(AwaitableState::Init),
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
            state: AtomicCell::new(AwaitableState::Init),
            value: (
                exc_type,
                exc_val.unwrap_or_none(vm),
                exc_tb.unwrap_or_none(vm),
            ),
        }
    }

    #[pymethod]
    fn aclose(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyAsyncGenAThrow {
        PyAsyncGenAThrow {
            ag: zelf,
            aclose: true,
            state: AtomicCell::new(AwaitableState::Init),
            value: (
                vm.ctx.exceptions.generator_exit.clone().into_object(),
                vm.ctx.none(),
                vm.ctx.none(),
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

#[pyclass(module = false, name = "async_generator_wrapped_value")]
#[derive(Debug)]
pub(crate) struct PyAsyncGenWrappedValue(pub PyObjectRef);
impl PyValue for PyAsyncGenWrappedValue {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.async_generator_wrapped_value
    }
}

#[pyimpl]
impl PyAsyncGenWrappedValue {}

impl PyAsyncGenWrappedValue {
    fn unbox(ag: &PyAsyncGen, val: PyResult, vm: &VirtualMachine) -> PyResult {
        if let Err(ref e) = val {
            if e.isinstance(&vm.ctx.exceptions.stop_async_iteration)
                || e.isinstance(&vm.ctx.exceptions.generator_exit)
            {
                ag.inner.closed.store(true);
            }
            ag.running_async.store(false);
        }
        let val = val?;

        match_class!(match val {
            val @ Self => {
                ag.running_async.store(false);
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

#[pyclass(module = false, name = "async_generator_asend")]
#[derive(Debug)]
pub(crate) struct PyAsyncGenASend {
    ag: PyAsyncGenRef,
    state: AtomicCell<AwaitableState>,
    value: PyObjectRef,
}

impl PyValue for PyAsyncGenASend {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.async_generator_asend
    }
}

#[pyimpl(with(PyIter))]
impl PyAsyncGenASend {
    #[pymethod(name = "__await__")]
    fn r#await(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyRef<Self> {
        zelf
    }

    #[pymethod]
    fn send(&self, val: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let val = match self.state.load() {
            AwaitableState::Closed => {
                return Err(vm.new_runtime_error(
                    "cannot reuse already awaited __anext__()/asend()".to_owned(),
                ))
            }
            AwaitableState::Iter => val, // already running, all good
            AwaitableState::Init => {
                if self.ag.running_async.load() {
                    return Err(vm.new_runtime_error(
                        "anext(): asynchronous generator is already running".to_owned(),
                    ));
                }
                self.ag.running_async.store(true);
                self.state.store(AwaitableState::Iter);
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
            self.close();
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
        if let AwaitableState::Closed = self.state.load() {
            return Err(
                vm.new_runtime_error("cannot reuse already awaited __anext__()/asend()".to_owned())
            );
        }

        let res = self.ag.inner.throw(
            exc_type,
            exc_val.unwrap_or_none(vm),
            exc_tb.unwrap_or_none(vm),
            vm,
        );
        let res = PyAsyncGenWrappedValue::unbox(&self.ag, res, vm);
        if res.is_err() {
            self.close();
        }
        res
    }

    #[pymethod]
    fn close(&self) {
        self.state.store(AwaitableState::Closed);
    }
}

impl PyIter for PyAsyncGenASend {
    fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        zelf.send(vm.ctx.none(), vm)
    }
}

#[pyclass(module = false, name = "async_generator_athrow")]
#[derive(Debug)]
pub(crate) struct PyAsyncGenAThrow {
    ag: PyAsyncGenRef,
    aclose: bool,
    state: AtomicCell<AwaitableState>,
    value: (PyObjectRef, PyObjectRef, PyObjectRef),
}

impl PyValue for PyAsyncGenAThrow {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.async_generator_athrow
    }
}

#[pyimpl(with(PyIter))]
impl PyAsyncGenAThrow {
    #[pymethod(name = "__await__")]
    fn r#await(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyRef<Self> {
        zelf
    }

    #[pymethod]
    fn send(&self, val: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        match self.state.load() {
            AwaitableState::Closed => {
                Err(vm
                    .new_runtime_error("cannot reuse already awaited aclose()/athrow()".to_owned()))
            }
            AwaitableState::Init => {
                if self.ag.running_async.load() {
                    self.state.store(AwaitableState::Closed);
                    let msg = if self.aclose {
                        "aclose(): asynchronous generator is already running"
                    } else {
                        "athrow(): asynchronous generator is already running"
                    };
                    return Err(vm.new_runtime_error(msg.to_owned()));
                }
                if self.ag.inner.closed() {
                    self.state.store(AwaitableState::Closed);
                    return Err(vm.new_exception_empty(vm.ctx.exceptions.stop_iteration.clone()));
                }
                if !vm.is_none(&val) {
                    return Err(vm.new_runtime_error(
                        "can't send non-None value to a just-started async generator".to_owned(),
                    ));
                }
                self.state.store(AwaitableState::Iter);
                self.ag.running_async.store(true);

                let (ty, val, tb) = self.value.clone();
                let ret = self.ag.inner.throw(ty, val, tb, vm);
                let ret = if self.aclose {
                    if self.ignored_close(&ret) {
                        Err(self.yield_close(vm))
                    } else {
                        ret
                    }
                } else {
                    PyAsyncGenWrappedValue::unbox(&self.ag, ret, vm)
                };
                ret.map_err(|e| self.check_error(e, vm))
            }
            AwaitableState::Iter => {
                let ret = self.ag.inner.send(val, vm);
                if self.aclose {
                    match ret {
                        Ok(v) if v.payload_is::<PyAsyncGenWrappedValue>() => {
                            Err(self.yield_close(vm))
                        }
                        Ok(v) => Ok(v),
                        Err(e) => Err(self.check_error(e, vm)),
                    }
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
            exc_val.unwrap_or_none(vm),
            exc_tb.unwrap_or_none(vm),
            vm,
        );
        let res = if self.aclose {
            if self.ignored_close(&ret) {
                Err(self.yield_close(vm))
            } else {
                ret
            }
        } else {
            PyAsyncGenWrappedValue::unbox(&self.ag, ret, vm)
        };
        res.map_err(|e| self.check_error(e, vm))
    }

    #[pymethod]
    fn close(&self) {
        self.state.store(AwaitableState::Closed);
    }

    fn ignored_close(&self, res: &PyResult) -> bool {
        res.as_ref()
            .map_or(false, |v| v.payload_is::<PyAsyncGenWrappedValue>())
    }
    fn yield_close(&self, vm: &VirtualMachine) -> PyBaseExceptionRef {
        self.ag.running_async.store(false);
        self.state.store(AwaitableState::Closed);
        vm.new_runtime_error("async generator ignored GeneratorExit".to_owned())
    }
    fn check_error(&self, exc: PyBaseExceptionRef, vm: &VirtualMachine) -> PyBaseExceptionRef {
        self.ag.running_async.store(false);
        self.state.store(AwaitableState::Closed);
        if self.aclose
            && (exc.isinstance(&vm.ctx.exceptions.stop_async_iteration)
                || exc.isinstance(&vm.ctx.exceptions.generator_exit))
        {
            vm.new_exception_empty(vm.ctx.exceptions.stop_iteration.clone())
        } else {
            exc
        }
    }
}

impl PyIter for PyAsyncGenAThrow {
    fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        zelf.send(vm.ctx.none(), vm)
    }
}

pub fn init(ctx: &PyContext) {
    PyAsyncGen::extend_class(ctx, &ctx.types.async_generator);
    PyAsyncGenASend::extend_class(ctx, &ctx.types.async_generator_asend);
    PyAsyncGenAThrow::extend_class(ctx, &ctx.types.async_generator_athrow);
}
