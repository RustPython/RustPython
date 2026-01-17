use super::{PyCode, PyGenericAlias, PyStrRef, PyType, PyTypeRef};
use crate::{
    AsObject, Context, Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    builtins::PyBaseExceptionRef,
    class::PyClassImpl,
    common::lock::PyMutex,
    coroutine::{Coro, warn_deprecated_throw_signature},
    frame::FrameRef,
    function::OptionalArg,
    object::{Traverse, TraverseFn},
    protocol::PyIterReturn,
    types::{Destructor, IterNext, Iterable, Representable, SelfIter},
};

use crossbeam_utils::atomic::AtomicCell;

#[pyclass(name = "async_generator", module = false, traverse = "manual")]
#[derive(Debug)]
pub struct PyAsyncGen {
    inner: Coro,
    running_async: AtomicCell<bool>,
    // whether hooks have been initialized
    ag_hooks_inited: AtomicCell<bool>,
    // ag_origin_or_finalizer - stores the finalizer callback
    ag_finalizer: PyMutex<Option<PyObjectRef>>,
}

unsafe impl Traverse for PyAsyncGen {
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        self.inner.traverse(tracer_fn);
        self.ag_finalizer.traverse(tracer_fn);
    }
}
type PyAsyncGenRef = PyRef<PyAsyncGen>;

impl PyPayload for PyAsyncGen {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.async_generator
    }
}

#[pyclass(flags(DISALLOW_INSTANTIATION), with(PyRef, Representable, Destructor))]
impl PyAsyncGen {
    pub const fn as_coro(&self) -> &Coro {
        &self.inner
    }

    pub fn new(frame: FrameRef, name: PyStrRef, qualname: PyStrRef) -> Self {
        Self {
            inner: Coro::new(frame, name, qualname),
            running_async: AtomicCell::new(false),
            ag_hooks_inited: AtomicCell::new(false),
            ag_finalizer: PyMutex::new(None),
        }
    }

    /// Initialize async generator hooks.
    /// Returns Ok(()) if successful, Err if firstiter hook raised an exception.
    fn init_hooks(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<()> {
        // = async_gen_init_hooks
        if zelf.ag_hooks_inited.load() {
            return Ok(());
        }

        zelf.ag_hooks_inited.store(true);

        // Get and store finalizer from VM
        let finalizer = vm.async_gen_finalizer.borrow().clone();
        if let Some(finalizer) = finalizer {
            *zelf.ag_finalizer.lock() = Some(finalizer);
        }

        // Call firstiter hook
        let firstiter = vm.async_gen_firstiter.borrow().clone();
        if let Some(firstiter) = firstiter {
            let obj: PyObjectRef = zelf.to_owned().into();
            firstiter.call((obj,), vm)?;
        }

        Ok(())
    }

    /// Call finalizer hook if set.
    fn call_finalizer(zelf: &Py<Self>, vm: &VirtualMachine) {
        let finalizer = zelf.ag_finalizer.lock().clone();
        if let Some(finalizer) = finalizer
            && !zelf.inner.closed.load()
        {
            // Create a strong reference for the finalizer call.
            // This keeps the object alive during the finalizer execution.
            let obj: PyObjectRef = zelf.to_owned().into();

            // Call the finalizer. Any exceptions are handled as unraisable.
            if let Err(e) = finalizer.call((obj,), vm) {
                vm.run_unraisable(e, Some("async generator finalizer".to_owned()), finalizer);
            }
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
    fn __qualname__(&self) -> PyStrRef {
        self.inner.qualname()
    }

    #[pygetset(setter)]
    fn set___qualname__(&self, qualname: PyStrRef) {
        self.inner.set_qualname(qualname)
    }

    #[pygetset]
    fn ag_await(&self, _vm: &VirtualMachine) -> Option<PyObjectRef> {
        self.inner.frame().yield_from_target()
    }
    #[pygetset]
    fn ag_frame(&self, _vm: &VirtualMachine) -> FrameRef {
        self.inner.frame()
    }
    #[pygetset]
    fn ag_running(&self, _vm: &VirtualMachine) -> bool {
        self.inner.running()
    }
    #[pygetset]
    fn ag_code(&self, _vm: &VirtualMachine) -> PyRef<PyCode> {
        self.inner.frame().code.clone()
    }

    #[pyclassmethod]
    fn __class_getitem__(cls: PyTypeRef, args: PyObjectRef, vm: &VirtualMachine) -> PyGenericAlias {
        PyGenericAlias::from_args(cls, args, vm)
    }
}

#[pyclass]
impl PyRef<PyAsyncGen> {
    #[pymethod]
    const fn __aiter__(self, _vm: &VirtualMachine) -> Self {
        self
    }

    #[pymethod]
    fn __anext__(self, vm: &VirtualMachine) -> PyResult<PyAsyncGenASend> {
        PyAsyncGen::init_hooks(&self, vm)?;
        Ok(PyAsyncGenASend {
            ag: self,
            state: AtomicCell::new(AwaitableState::Init),
            value: vm.ctx.none(),
        })
    }

    #[pymethod]
    fn asend(self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyAsyncGenASend> {
        PyAsyncGen::init_hooks(&self, vm)?;
        Ok(PyAsyncGenASend {
            ag: self,
            state: AtomicCell::new(AwaitableState::Init),
            value,
        })
    }

    #[pymethod]
    fn athrow(
        self,
        exc_type: PyObjectRef,
        exc_val: OptionalArg,
        exc_tb: OptionalArg,
        vm: &VirtualMachine,
    ) -> PyResult<PyAsyncGenAThrow> {
        PyAsyncGen::init_hooks(&self, vm)?;
        Ok(PyAsyncGenAThrow {
            ag: self,
            aclose: false,
            state: AtomicCell::new(AwaitableState::Init),
            value: (
                exc_type,
                exc_val.unwrap_or_none(vm),
                exc_tb.unwrap_or_none(vm),
            ),
        })
    }

    #[pymethod]
    fn aclose(self, vm: &VirtualMachine) -> PyResult<PyAsyncGenAThrow> {
        PyAsyncGen::init_hooks(&self, vm)?;
        Ok(PyAsyncGenAThrow {
            ag: self,
            aclose: true,
            state: AtomicCell::new(AwaitableState::Init),
            value: (
                vm.ctx.exceptions.generator_exit.to_owned().into(),
                vm.ctx.none(),
                vm.ctx.none(),
            ),
        })
    }
}

impl Representable for PyAsyncGen {
    #[inline]
    fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
        Ok(zelf.inner.repr(zelf.as_object(), zelf.get_id(), vm))
    }
}

#[pyclass(
    module = false,
    name = "async_generator_wrapped_value",
    traverse = "manual"
)]
#[derive(Debug)]
pub(crate) struct PyAsyncGenWrappedValue(pub PyObjectRef);

unsafe impl Traverse for PyAsyncGenWrappedValue {
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        self.0.traverse(tracer_fn);
    }
}

impl PyPayload for PyAsyncGenWrappedValue {
    #[inline]
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
            ag.inner.closed.store(true);
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

#[pyclass(module = false, name = "async_generator_asend", traverse = "manual")]
#[derive(Debug)]
pub(crate) struct PyAsyncGenASend {
    ag: PyAsyncGenRef,
    state: AtomicCell<AwaitableState>,
    value: PyObjectRef,
}

unsafe impl Traverse for PyAsyncGenASend {
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        self.ag.traverse(tracer_fn);
        self.value.traverse(tracer_fn);
    }
}

impl PyPayload for PyAsyncGenASend {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.async_generator_asend
    }
}

#[pyclass(with(IterNext, Iterable))]
impl PyAsyncGenASend {
    #[pymethod(name = "__await__")]
    const fn r#await(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyRef<Self> {
        zelf
    }

    #[pymethod]
    fn send(&self, val: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let val = match self.state.load() {
            AwaitableState::Closed => {
                return Err(
                    vm.new_runtime_error("cannot reuse already awaited __anext__()/asend()")
                );
            }
            AwaitableState::Iter => val, // already running, all good
            AwaitableState::Init => {
                if self.ag.running_async.load() {
                    return Err(
                        vm.new_runtime_error("anext(): asynchronous generator is already running")
                    );
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
            return Err(vm.new_runtime_error("cannot reuse already awaited __anext__()/asend()"));
        }

        warn_deprecated_throw_signature(&exc_val, &exc_tb, vm)?;
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

#[pyclass(module = false, name = "async_generator_athrow", traverse = "manual")]
#[derive(Debug)]
pub(crate) struct PyAsyncGenAThrow {
    ag: PyAsyncGenRef,
    aclose: bool,
    state: AtomicCell<AwaitableState>,
    value: (PyObjectRef, PyObjectRef, PyObjectRef),
}

unsafe impl Traverse for PyAsyncGenAThrow {
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        self.ag.traverse(tracer_fn);
        self.value.traverse(tracer_fn);
    }
}

impl PyPayload for PyAsyncGenAThrow {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.async_generator_athrow
    }
}

#[pyclass(with(IterNext, Iterable))]
impl PyAsyncGenAThrow {
    #[pymethod(name = "__await__")]
    const fn r#await(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyRef<Self> {
        zelf
    }

    #[pymethod]
    fn send(&self, val: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        match self.state.load() {
            AwaitableState::Closed => {
                Err(vm.new_runtime_error("cannot reuse already awaited aclose()/athrow()"))
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
                        "can't send non-None value to a just-started async generator",
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
                        Ok(PyIterReturn::Return(v))
                            if v.downcastable::<PyAsyncGenWrappedValue>() =>
                        {
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
        warn_deprecated_throw_signature(&exc_val, &exc_tb, vm)?;
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
            PyIterReturn::Return(obj) => obj.downcastable::<PyAsyncGenWrappedValue>(),
            PyIterReturn::StopIteration(_) => false,
        })
    }
    fn yield_close(&self, vm: &VirtualMachine) -> PyBaseExceptionRef {
        self.ag.running_async.store(false);
        self.ag.inner.closed.store(true);
        self.state.store(AwaitableState::Closed);
        vm.new_runtime_error("async generator ignored GeneratorExit")
    }
    fn check_error(&self, exc: PyBaseExceptionRef, vm: &VirtualMachine) -> PyBaseExceptionRef {
        self.ag.running_async.store(false);
        self.ag.inner.closed.store(true);
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

/// Awaitable wrapper for anext() builtin with default value.
/// When StopAsyncIteration is raised, it converts it to StopIteration(default).
#[pyclass(module = false, name = "anext_awaitable", traverse = "manual")]
#[derive(Debug)]
pub struct PyAnextAwaitable {
    wrapped: PyObjectRef,
    default_value: PyObjectRef,
    state: AtomicCell<AwaitableState>,
}

unsafe impl Traverse for PyAnextAwaitable {
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        self.wrapped.traverse(tracer_fn);
        self.default_value.traverse(tracer_fn);
    }
}

impl PyPayload for PyAnextAwaitable {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.anext_awaitable
    }
}

#[pyclass(with(IterNext, Iterable))]
impl PyAnextAwaitable {
    pub fn new(wrapped: PyObjectRef, default_value: PyObjectRef) -> Self {
        Self {
            wrapped,
            default_value,
            state: AtomicCell::new(AwaitableState::Init),
        }
    }

    #[pymethod(name = "__await__")]
    fn r#await(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyRef<Self> {
        zelf
    }

    fn check_closed(&self, vm: &VirtualMachine) -> PyResult<()> {
        if let AwaitableState::Closed = self.state.load() {
            return Err(vm.new_runtime_error("cannot reuse already awaited __anext__()/asend()"));
        }
        Ok(())
    }

    /// Get the awaitable iterator from wrapped object.
    // = anextawaitable_getiter.
    fn get_awaitable_iter(&self, vm: &VirtualMachine) -> PyResult {
        use crate::builtins::PyCoroutine;
        use crate::protocol::PyIter;

        let wrapped = &self.wrapped;

        // If wrapped is already an async_generator_asend, it's an iterator
        if wrapped.class().is(vm.ctx.types.async_generator_asend)
            || wrapped.class().is(vm.ctx.types.async_generator_athrow)
        {
            return Ok(wrapped.clone());
        }

        // _PyCoro_GetAwaitableIter equivalent
        let awaitable = if wrapped.class().is(vm.ctx.types.coroutine_type) {
            // Coroutine - get __await__ later
            wrapped.clone()
        } else {
            // Try to get __await__ method
            if let Some(await_method) = vm.get_method(wrapped.clone(), identifier!(vm, __await__)) {
                await_method?.call((), vm)?
            } else {
                return Err(vm.new_type_error(format!(
                    "object {} can't be used in 'await' expression",
                    wrapped.class().name()
                )));
            }
        };

        // If awaitable is a coroutine, get its __await__
        if awaitable.class().is(vm.ctx.types.coroutine_type) {
            let coro_await = vm.call_method(&awaitable, "__await__", ())?;
            // Check that __await__ returned an iterator
            if !PyIter::check(&coro_await) {
                return Err(vm.new_type_error("__await__ returned a non-iterable"));
            }
            return Ok(coro_await);
        }

        // Check the result is an iterator, not a coroutine
        if awaitable.downcast_ref::<PyCoroutine>().is_some() {
            return Err(vm.new_type_error("__await__() returned a coroutine"));
        }

        // Check that the result is an iterator
        if !PyIter::check(&awaitable) {
            return Err(vm.new_type_error(format!(
                "__await__() returned non-iterator of type '{}'",
                awaitable.class().name()
            )));
        }

        Ok(awaitable)
    }

    #[pymethod]
    fn send(&self, val: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.check_closed(vm)?;
        self.state.store(AwaitableState::Iter);
        let awaitable = self.get_awaitable_iter(vm)?;
        let result = vm.call_method(&awaitable, "send", (val,));
        self.handle_result(result, vm)
    }

    #[pymethod]
    fn throw(
        &self,
        exc_type: PyObjectRef,
        exc_val: OptionalArg,
        exc_tb: OptionalArg,
        vm: &VirtualMachine,
    ) -> PyResult {
        self.check_closed(vm)?;
        warn_deprecated_throw_signature(&exc_val, &exc_tb, vm)?;
        self.state.store(AwaitableState::Iter);
        let awaitable = self.get_awaitable_iter(vm)?;
        let result = vm.call_method(
            &awaitable,
            "throw",
            (
                exc_type,
                exc_val.unwrap_or_none(vm),
                exc_tb.unwrap_or_none(vm),
            ),
        );
        self.handle_result(result, vm)
    }

    #[pymethod]
    fn close(&self, vm: &VirtualMachine) -> PyResult<()> {
        self.state.store(AwaitableState::Closed);
        if let Ok(awaitable) = self.get_awaitable_iter(vm) {
            let _ = vm.call_method(&awaitable, "close", ());
        }
        Ok(())
    }

    /// Convert StopAsyncIteration to StopIteration(default_value)
    fn handle_result(&self, result: PyResult, vm: &VirtualMachine) -> PyResult {
        match result {
            Ok(value) => Ok(value),
            Err(exc) if exc.fast_isinstance(vm.ctx.exceptions.stop_async_iteration) => {
                Err(vm.new_stop_iteration(Some(self.default_value.clone())))
            }
            Err(exc) => Err(exc),
        }
    }
}

impl SelfIter for PyAnextAwaitable {}
impl IterNext for PyAnextAwaitable {
    fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        PyIterReturn::from_pyresult(zelf.send(vm.ctx.none(), vm), vm)
    }
}

/// _PyGen_Finalize for async generators
impl Destructor for PyAsyncGen {
    fn del(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<()> {
        // Generator is already closed, nothing to do
        if zelf.inner.closed.load() {
            return Ok(());
        }

        // Call the async generator finalizer hook if set.
        Self::call_finalizer(zelf, vm);

        Ok(())
    }
}

pub fn init(ctx: &Context) {
    PyAsyncGen::extend_class(ctx, ctx.types.async_generator);
    PyAsyncGenASend::extend_class(ctx, ctx.types.async_generator_asend);
    PyAsyncGenAThrow::extend_class(ctx, ctx.types.async_generator_athrow);
    PyAnextAwaitable::extend_class(ctx, ctx.types.anext_awaitable);
}
