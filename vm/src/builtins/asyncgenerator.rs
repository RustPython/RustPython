use super::{PyCode, PyGenericAlias, PyStrRef, PyType, PyTypeRef};
use crate::{
    AsObject, Context, Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    builtins::PyBaseExceptionRef,
    class::PyClassImpl,
    coroutine::Coro,
    frame::FrameRef,
    function::OptionalArg,
    protocol::PyIterReturn,
    types::{IterNext, Iterable, Representable, SelfIter, Unconstructible},
};

use crossbeam_utils::atomic::AtomicCell;

#[pyclass(name = "async_generator", module = false)]
#[derive(Debug)]
pub struct PyAsyncGen {
    inner: Coro,
    running_async: AtomicCell<bool>,
}
type PyAsyncGenRef = PyRef<PyAsyncGen>;

impl PyPayload for PyAsyncGen {
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.async_generator
    }
}

#[pyclass(with(PyRef, Unconstructible, Representable))]
impl PyAsyncGen {
    pub fn as_coro(&self) -> &Coro {
        &self.inner
    }

    pub fn new(frame: FrameRef, name: PyStrRef) -> Self {
        PyAsyncGen {
            inner: Coro::new(frame, name),
            running_async: AtomicCell::new(false),
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
    fn ag_await(&self, _vm: &VirtualMachine) -> Option<PyObjectRef> {
        self.inner.frame().and_then(|x| x.yield_from_target())
    }
    #[pygetset]
    fn ag_frame(&self, _vm: &VirtualMachine) -> Option<FrameRef> {
        self.inner.frame()
    }
    #[pygetset]
    fn ag_running(&self, _vm: &VirtualMachine) -> bool {
        self.inner.running()
    }
    #[pygetset]
    fn ag_code(&self, _vm: &VirtualMachine) -> Option<PyRef<PyCode>> {
        self.inner.frame().map(|x| x.code.clone())
    }

    #[pyclassmethod(magic)]
    fn class_getitem(cls: PyTypeRef, args: PyObjectRef, vm: &VirtualMachine) -> PyGenericAlias {
        PyGenericAlias::new(cls, args, vm)
    }
}

#[pyclass]
impl PyRef<PyAsyncGen> {
    #[pymethod(magic)]
    fn aiter(self, _vm: &VirtualMachine) -> PyRef<PyAsyncGen> {
        self
    }

    #[pymethod(magic)]
    fn anext(self, vm: &VirtualMachine) -> PyAsyncGenASend {
        Self::asend(self, vm.ctx.none(), vm)
    }

    #[pymethod]
    fn asend(self, value: PyObjectRef, _vm: &VirtualMachine) -> PyAsyncGenASend {
        PyAsyncGenASend {
            ag: self,
            state: AtomicCell::new(AwaitableState::Init),
            value,
        }
    }

    #[pymethod]
    fn athrow(
        self,
        exc_type: PyObjectRef,
        exc_val: OptionalArg,
        exc_tb: OptionalArg,
        vm: &VirtualMachine,
    ) -> PyAsyncGenAThrow {
        PyAsyncGenAThrow {
            ag: self,
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
    fn aclose(self, vm: &VirtualMachine) -> PyAsyncGenAThrow {
        PyAsyncGenAThrow {
            ag: self,
            aclose: true,
            state: AtomicCell::new(AwaitableState::Init),
            value: (
                vm.ctx.exceptions.generator_exit.to_owned().into(),
                vm.ctx.none(),
                vm.ctx.none(),
            ),
        }
    }
}

impl Representable for PyAsyncGen {
    #[inline]
    fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
        Ok(zelf.inner.repr(zelf.as_object(), zelf.get_id(), vm))
    }
}

impl Unconstructible for PyAsyncGen {}

#[pyclass(module = false, name = "async_generator_wrapped_value")]
#[derive(Debug)]
pub(crate) struct PyAsyncGenWrappedValue(pub PyObjectRef);
impl PyPayload for PyAsyncGenWrappedValue {
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.async_generator_wrapped_value
    }
}

#[pyclass]
impl PyAsyncGenWrappedValue {}

impl PyAsyncGenWrappedValue {
    fn unbox(ag: &PyAsyncGen, val: PyResult<PyIterReturn>, vm: &VirtualMachine) -> PyResult {
        let (closed, async_done) = match &val {
            Ok(PyIterReturn::StopIteration(_)) => (true, true),
            Err(e) if e.fast_isinstance(vm.ctx.exceptions.generator_exit) => (true, true),
            Err(_) => (false, true),
            _ => (false, false),
        };
        if closed {
            // ag.inner.closed.store(true);
            ag.inner.set_close();
        }
        if async_done {
            ag.running_async.store(false);
        }
        let val = val?.into_async_pyresult(vm)?;
        match_class!(match val {
            val @ Self => {
                ag.running_async.store(false);
                Err(vm.new_stop_iteration(Some(val.0.clone())))
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

impl PyPayload for PyAsyncGenASend {
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.async_generator_asend
    }
}

#[pyclass(with(IterNext, Iterable))]
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
                ));
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
        let res = self.ag.inner.send(self.ag.as_object(), val, vm);
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
            self.ag.as_object(),
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

impl SelfIter for PyAsyncGenASend {}
impl IterNext for PyAsyncGenASend {
    fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        PyIterReturn::from_pyresult(zelf.send(vm.ctx.none(), vm), vm)
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

impl PyPayload for PyAsyncGenAThrow {
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.async_generator_athrow
    }
}

#[pyclass(with(IterNext, Iterable))]
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
                    return Err(vm.new_stop_iteration(None));
                }
                if !vm.is_none(&val) {
                    return Err(vm.new_runtime_error(
                        "can't send non-None value to a just-started async generator".to_owned(),
                    ));
                }
                self.state.store(AwaitableState::Iter);
                self.ag.running_async.store(true);

                let (ty, val, tb) = self.value.clone();
                let ret = self.ag.inner.throw(self.ag.as_object(), ty, val, tb, vm);
                let ret = if self.aclose {
                    if self.ignored_close(&ret) {
                        Err(self.yield_close(vm))
                    } else {
                        ret.and_then(|o| o.into_async_pyresult(vm))
                    }
                } else {
                    PyAsyncGenWrappedValue::unbox(&self.ag, ret, vm)
                };
                ret.map_err(|e| self.check_error(e, vm))
            }
            AwaitableState::Iter => {
                let ret = self.ag.inner.send(self.ag.as_object(), val, vm);
                if self.aclose {
                    match ret {
                        Ok(PyIterReturn::Return(v)) if v.payload_is::<PyAsyncGenWrappedValue>() => {
                            Err(self.yield_close(vm))
                        }
                        other => other
                            .and_then(|o| o.into_async_pyresult(vm))
                            .map_err(|e| self.check_error(e, vm)),
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
            self.ag.as_object(),
            exc_type,
            exc_val.unwrap_or_none(vm),
            exc_tb.unwrap_or_none(vm),
            vm,
        );
        let res = if self.aclose {
            if self.ignored_close(&ret) {
                Err(self.yield_close(vm))
            } else {
                ret.and_then(|o| o.into_async_pyresult(vm))
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

    fn ignored_close(&self, res: &PyResult<PyIterReturn>) -> bool {
        res.as_ref().is_ok_and(|v| match v {
            PyIterReturn::Return(obj) => obj.payload_is::<PyAsyncGenWrappedValue>(),
            PyIterReturn::StopIteration(_) => false,
        })
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
            && (exc.fast_isinstance(vm.ctx.exceptions.stop_async_iteration)
                || exc.fast_isinstance(vm.ctx.exceptions.generator_exit))
        {
            vm.new_stop_iteration(None)
        } else {
            exc
        }
    }
}

impl SelfIter for PyAsyncGenAThrow {}
impl IterNext for PyAsyncGenAThrow {
    fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        PyIterReturn::from_pyresult(zelf.send(vm.ctx.none(), vm), vm)
    }
}

pub fn init(ctx: &Context) {
    PyAsyncGen::extend_class(ctx, ctx.types.async_generator);
    PyAsyncGenASend::extend_class(ctx, ctx.types.async_generator_asend);
    PyAsyncGenAThrow::extend_class(ctx, ctx.types.async_generator_athrow);
}
