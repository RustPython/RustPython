use crate::{
    AsObject, Py, PyObject, PyObjectRef, PyResult, TryFromObject, VirtualMachine,
    builtins::PyStrRef,
    common::lock::PyMutex,
    exceptions::types::PyBaseException,
    frame::{ExecutionResult, FrameObject, FrameObjectRef, FrameOwner},
    function::OptionalArg,
    object::{PyAtomicRef, Traverse, TraverseFn},
    protocol::PyIterReturn,
};
use crossbeam_utils::atomic::AtomicCell;

impl ExecutionResult {
    /// Turn an ExecutionResult into a PyResult that would be returned from a generator or coroutine
    fn into_iter_return(self, vm: &VirtualMachine) -> PyIterReturn {
        match self {
            Self::Yield(value) => PyIterReturn::Return(value),
            Self::Return(value) => {
                let arg = if vm.is_none(&value) {
                    None
                } else {
                    Some(value)
                };
                PyIterReturn::StopIteration(arg)
            }
            Self::TailCall => unreachable!("TailCall in generator/coroutine"),
        }
    }
}

#[derive(Debug)]
pub struct Coro {
    frame: FrameObjectRef,
    pub closed: AtomicCell<bool>, // TODO: https://github.com/RustPython/RustPython/pull/3183#discussion_r720560652
    running: AtomicCell<bool>,
    // code
    // _weakreflist
    name: PyMutex<PyStrRef>,
    qualname: PyMutex<PyStrRef>,
    exception: PyAtomicRef<Option<PyBaseException>>, // exc_state
}

unsafe impl Traverse for Coro {
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        self.frame.traverse(tracer_fn);
        self.name.traverse(tracer_fn);
        self.qualname.traverse(tracer_fn);
        if let Some(exc) = self.exception.deref() {
            exc.traverse(tracer_fn);
        }
    }
}

/// An exclusive claim on a generator's frame, released when dropped.
///
/// Only the holder may look at the frame or resume it. Resuming decides from
/// the frame state whether the sent value goes on the value stack, so a state
/// read taken before the claim can be answered by a frame that another thread
/// then advances: resuming it leaves the stack short of what the code after
/// the yield pops.
struct RunningGuard<'a>(&'a Coro);

impl Drop for RunningGuard<'_> {
    fn drop(&mut self) {
        self.0.running.store(false);
    }
}

fn gen_name(jen: &PyObject, vm: &VirtualMachine) -> &'static str {
    let typ = jen.class();
    if typ.is(vm.ctx.types.coroutine_type) {
        "coroutine"
    } else if typ.is(vm.ctx.types.async_generator) {
        "async generator"
    } else {
        "generator"
    }
}

impl Coro {
    pub fn new(frame: FrameObjectRef, name: PyStrRef, qualname: PyStrRef) -> Self {
        Self {
            frame,
            closed: AtomicCell::new(false),
            running: AtomicCell::new(false),
            exception: PyAtomicRef::from(None),
            name: PyMutex::new(name),
            qualname: PyMutex::new(qualname),
        }
    }

    /// Free the finished frame's locals and stack, unless a frame object has
    /// escaped (e.g. through an `f_locals` proxy or `sys._getframe`). An
    /// escaped frame husk owns its heap-resident locals and must keep them
    /// readable after the generator closes.
    fn clear_frame_locals_on_close(&self) {
        // Keep locals alive if a durable frame reference escaped (e.g. through
        // an `f_locals` proxy or `sys._getframe`): that reference now owns the
        // heap-resident locals and must keep them readable after close,
        // matching `take_ownership`.
        if !self.frame.has_escaped() {
            self.frame.clear_locals_and_stack();
        }
    }

    /// Retire the generator if the frame it just ran came to an end. The claim
    /// is still held, so a thread waiting for it cannot resume a frame that has
    /// already finished.
    fn maybe_close(&self, res: &PyResult<ExecutionResult>, _claim: &RunningGuard<'_>) {
        match res {
            Ok(ExecutionResult::Return(_)) | Err(_) => {
                self.closed.store(true);
                // FrameObject is no longer suspended; allow frame.clear() to succeed.
                self.frame.iframe().owner.store(
                    FrameOwner::FrameObject as i8,
                    core::sync::atomic::Ordering::Release,
                );
                // Completed generators/coroutines should not keep their locals
                // alive while the wrapper object itself remains referenced.
                self.clear_frame_locals_on_close();
            }
            Ok(ExecutionResult::Yield(_)) => {}
            Ok(ExecutionResult::TailCall) => unreachable!("TailCall in generator/coroutine"),
        }
    }

    /// Take the frame for this thread, or report that another thread holds it.
    ///
    /// What the resume depends on -- whether the generator is closed, and
    /// whether it has started -- has to be read from here onwards.
    fn claim(&self, jen: &PyObject, vm: &VirtualMachine) -> PyResult<RunningGuard<'_>> {
        if self.running.compare_exchange(false, true).is_err() {
            return Err(vm.new_value_error(format!("{} already executing", gen_name(jen, vm))));
        }
        Ok(RunningGuard(self))
    }

    fn run_claimed<F>(
        &self,
        _claim: &RunningGuard<'_>,
        vm: &VirtualMachine,
        func: F,
    ) -> PyResult<ExecutionResult>
    where
        F: FnOnce(&Py<FrameObject>) -> PyResult<ExecutionResult>,
    {
        // SAFETY: the claim guarantees exclusive access
        let gen_exc = unsafe { self.exception.swap(None) };
        let exception_ptr = &self.exception as *const PyAtomicRef<Option<PyBaseException>>;

        vm.resume_gen_frame(&self.frame, gen_exc, |f| {
            let result = func(f);
            // SAFETY: exclusive access guaranteed by the claim
            let _old = unsafe { (*exception_ptr).swap(vm.current_exception()) };
            result
        })
    }

    fn finalize_send_result(
        &self,
        result: PyResult<ExecutionResult>,
        jen: &PyObject,
        vm: &VirtualMachine,
    ) -> PyResult<PyIterReturn> {
        match result {
            Ok(exec_res) => Ok(exec_res.into_iter_return(vm)),
            Err(e) => {
                if e.fast_isinstance(vm.ctx.exceptions.stop_iteration) {
                    let err =
                        vm.new_runtime_error(format!("{} raised StopIteration", gen_name(jen, vm)));
                    // PEP 479: chain __context__ as well as __cause__ to match
                    // CPython, which sets both to the original StopIteration.
                    err.set___context__(Some(e.clone()));
                    err.set___cause__(Some(e));
                    Err(err)
                } else if jen.class().is(vm.ctx.types.async_generator)
                    && e.fast_isinstance(vm.ctx.exceptions.stop_async_iteration)
                {
                    let err = vm.new_runtime_error("async generator raised StopAsyncIteration");
                    err.set___context__(Some(e.clone()));
                    err.set___cause__(Some(e));
                    Err(err)
                } else {
                    Err(e)
                }
            }
        }
    }

    pub(crate) fn send_none(&self, jen: &PyObject, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        if self.closed.load() {
            return Ok(PyIterReturn::StopIteration(None));
        }
        let claim = self.claim(jen, vm)?;
        // The generator can have run to its end in the meantime.
        if self.closed.load() {
            return Ok(PyIterReturn::StopIteration(None));
        }
        let value = if self.frame.lasti() > 0 {
            Some(vm.ctx.none())
        } else {
            None
        };
        let result = self.run_claimed(&claim, vm, |f| f.resume(value, vm));
        self.maybe_close(&result, &claim);
        drop(claim);
        self.finalize_send_result(result, jen, vm)
    }

    pub fn send(
        &self,
        jen: &PyObject,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyIterReturn> {
        if self.closed.load() {
            return Ok(PyIterReturn::StopIteration(None));
        }
        let claim = self.claim(jen, vm)?;
        // The generator can have run to its end in the meantime.
        if self.closed.load() {
            return Ok(PyIterReturn::StopIteration(None));
        }
        let value = if self.frame.lasti() > 0 {
            Some(value)
        } else if !vm.is_none(&value) {
            return Err(vm.new_type_error(format!(
                "can't send non-None value to a just-started {}",
                gen_name(jen, vm),
            )));
        } else {
            None
        };
        let result = self.run_claimed(&claim, vm, |f| f.resume(value, vm));
        self.maybe_close(&result, &claim);
        drop(claim);
        self.finalize_send_result(result, jen, vm)
    }

    pub fn throw(
        &self,
        jen: &PyObject,
        exc_type: PyObjectRef,
        exc_val: PyObjectRef,
        exc_tb: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyIterReturn> {
        // Validate throw arguments (_gen_throw)
        if exc_type.fast_isinstance(vm.ctx.exceptions.base_exception_type) && !vm.is_none(&exc_val)
        {
            return Err(vm.new_type_error("instance exception may not have a separate value"));
        }
        if !vm.is_none(&exc_tb) && !exc_tb.fast_isinstance(vm.ctx.types.traceback_type) {
            return Err(vm.new_type_error("throw() third argument must be a traceback object"));
        }
        if self.closed.load() {
            return Err(vm.normalize_exception(exc_type, exc_val, exc_tb)?);
        }
        // Validate exception type before entering generator context.
        // Invalid types propagate to caller without closing the generator.
        crate::exceptions::ExceptionCtor::try_from_object(vm, exc_type.clone())?;
        let claim = self.claim(jen, vm)?;
        // The generator can have run to its end in the meantime. Normalizing
        // runs the exception's constructor, so let the claim go first.
        if self.closed.load() {
            drop(claim);
            return Err(vm.normalize_exception(exc_type, exc_val, exc_tb)?);
        }
        let result = self.run_claimed(&claim, vm, |f| f.gen_throw(vm, exc_type, exc_val, exc_tb));
        self.maybe_close(&result, &claim);
        drop(claim);
        Ok(result?.into_iter_return(vm))
    }

    pub fn close(&self, jen: &PyObject, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        if self.closed.load() {
            return Ok(vm.ctx.none());
        }
        let claim = self.claim(jen, vm)?;
        // The generator can have run to its end in the meantime.
        if self.closed.load() {
            return Ok(vm.ctx.none());
        }
        // If generator hasn't started (FRAME_CREATED), just mark as closed
        if self.frame.lasti() == 0 {
            self.closed.store(true);
            return Ok(vm.ctx.none());
        }
        let result = self.run_claimed(&claim, vm, |f| {
            f.gen_throw(
                vm,
                vm.ctx.exceptions.generator_exit.to_owned().into(),
                vm.ctx.none(),
                vm.ctx.none(),
            )
        });
        self.closed.store(true);
        // Release frame locals and stack to free references held by the
        // closed generator, matching gen_send_ex2 with close_on_completion.
        self.clear_frame_locals_on_close();
        drop(claim);
        match result {
            Ok(ExecutionResult::Yield(_)) => {
                Err(vm.new_runtime_error(format!("{} ignored GeneratorExit", gen_name(jen, vm))))
            }
            Err(e) if !is_gen_exit(&e, vm) => Err(e),
            Ok(ExecutionResult::Return(value)) => Ok(value),
            _ => Ok(vm.ctx.none()),
        }
    }

    pub fn suspended(&self) -> bool {
        !self.closed.load() && !self.running.load() && self.frame.lasti() > 0
    }

    pub fn running(&self) -> bool {
        self.running.load()
    }

    pub fn closed(&self) -> bool {
        self.closed.load()
    }

    pub fn frame(&self) -> FrameObjectRef {
        self.frame.clone()
    }

    pub fn name(&self) -> PyStrRef {
        self.name.lock().clone()
    }

    pub fn set_name(&self, name: PyStrRef) {
        *self.name.lock() = name;
    }

    pub fn qualname(&self) -> PyStrRef {
        self.qualname.lock().clone()
    }

    pub fn set_qualname(&self, qualname: PyStrRef) {
        *self.qualname.lock() = qualname;
    }

    pub fn repr(&self, jen: &PyObject, id: usize, vm: &VirtualMachine) -> String {
        let qualname = self.qualname();
        format!(
            "<{} object {} at {:#x}>",
            gen_name(jen, vm),
            qualname.as_wtf8(),
            id
        )
    }
}

pub(crate) fn is_gen_exit(exc: &Py<PyBaseException>, vm: &VirtualMachine) -> bool {
    exc.fast_isinstance(vm.ctx.exceptions.generator_exit)
}

/// Get an awaitable iterator from an object.
///
/// Returns the object itself if it's a coroutine or iterable coroutine (generator with
/// CO_ITERABLE_COROUTINE flag). Otherwise calls `__await__()` and validates the result.
pub(crate) fn get_awaitable_iter(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    use crate::builtins::{PyCoroutine, PyGenerator};
    use crate::protocol::PyIter;

    if obj.downcastable::<PyCoroutine>()
        || obj.downcast_ref::<PyGenerator>().is_some_and(|g| {
            g.as_coro()
                .frame()
                .iframe()
                .code()
                .flags
                .contains(crate::bytecode::CodeFlags::ITERABLE_COROUTINE)
        })
    {
        return Ok(obj);
    }

    if let Some(await_method) = vm.get_method(obj.clone(), identifier!(vm, __await__)) {
        let result = await_method?.call((), vm)?;
        // __await__() must NOT return a coroutine (PEP 492)
        if result.downcastable::<PyCoroutine>()
            || result.downcast_ref::<PyGenerator>().is_some_and(|g| {
                g.as_coro()
                    .frame()
                    .iframe()
                    .code()
                    .flags
                    .contains(crate::bytecode::CodeFlags::ITERABLE_COROUTINE)
            })
        {
            return Err(vm.new_type_error("__await__() returned a coroutine"));
        }
        if !PyIter::check(&result) {
            return Err(vm.new_type_error(format!(
                "__await__() returned non-iterator of type '{}'",
                result.class().name()
            )));
        }
        return Ok(result);
    }

    Err(vm.new_type_error(format!("'{}' object can't be awaited", obj.class().name())))
}

/// Emit DeprecationWarning for the deprecated 3-argument throw() signature.
pub(crate) fn warn_deprecated_throw_signature(
    exc_val: &OptionalArg,
    exc_tb: &OptionalArg,
    vm: &VirtualMachine,
) -> PyResult<()> {
    if exc_val.is_present() || exc_tb.is_present() {
        crate::warn::warn(
            vm.ctx
                .new_str(
                    "the (type, val, tb) signature of throw() is deprecated, \
                 use throw(val) instead",
                )
                .into(),
            Some(vm.ctx.exceptions.deprecation_warning.to_owned()),
            1,
            None,
            vm,
        )?;
    }
    Ok(())
}
