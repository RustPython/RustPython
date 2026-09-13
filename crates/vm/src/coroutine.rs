use crate::{
    AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject, VirtualMachine,
    builtins::{PyCode, PyStrRef, PyTraceback, PyTupleRef},
    common::lock::PyMutex,
    exceptions::types::PyBaseException,
    frame::{ExecutionResult, FrameObject, FrameObjectRef, FrameOwner, InterpreterFrame},
    function::OptionalArg,
    object::{PyAtomicRef, Traverse, TraverseFn},
    protocol::PyIterReturn,
};
use core::sync::atomic::Ordering;
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
    /// Generator iframe, like `gi_iframe`. Cleared to `None` by take_ownership
    /// the same way `frame_obj` is stolen from the iframe.
    frame: PyAtomicRef<Option<FrameObject>>,
    /// `f_executable`; survives `_PyFrame_ClearExceptCode`.
    code: PyRef<PyCode>,
    pub closed: AtomicCell<bool>, // TODO: https://github.com/RustPython/RustPython/pull/3183#discussion_r720560652
    running: AtomicCell<bool>,
    // _weakreflist
    name: PyMutex<PyStrRef>,
    qualname: PyMutex<PyStrRef>,
    exception: PyAtomicRef<Option<PyBaseException>>, // exc_state
}

unsafe impl Traverse for Coro {
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        if let Some(frame) = self.frame.deref() {
            tracer_fn(frame.as_object());
        }
        self.code.traverse(tracer_fn);
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
        let code = frame.iframe().code().to_owned();
        frame.as_object().mark_cache_published();
        Self {
            frame: Some(frame).into(),
            code,
            closed: AtomicCell::new(false),
            running: AtomicCell::new(false),
            exception: PyAtomicRef::from(None),
            name: PyMutex::new(name),
            qualname: PyMutex::new(qualname),
        }
    }

    /// `_PyFrame_ClearExceptCode`. Steal the frame slot first, then
    /// clear locals only if this was the last reference.
    fn clear_except_code(&self) {
        let Some(frame) = (unsafe { self.frame.swap(None) }) else {
            return;
        };
        frame.clear_generator();
        if frame.as_object().strong_count() == 1 {
            frame.clear_locals_and_stack();
        } else {
            frame.iframe().owner.store(
                FrameOwner::FrameObject as i8,
                core::sync::atomic::Ordering::Release,
            );
        }
    }

    /// Retire the generator if the frame it just ran came to an end. The claim
    /// is still held, so a thread waiting for it cannot resume a frame that has
    /// already finished.
    fn maybe_close(&self, res: &PyResult<ExecutionResult>, _claim: &RunningGuard<'_>) {
        match res {
            Ok(ExecutionResult::Return(_)) | Err(_) => {
                self.closed.store(true);
                self.clear_except_code();
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

        let frame = self.frame();
        vm.resume_gen_frame(&frame, gen_exc, |f| {
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
                    // PEP 479: chain __context__ as well as __cause__ to the
                    // original StopIteration.
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
            return Self::send_when_closed(jen, vm);
        }
        let claim = self.claim(jen, vm)?;
        // The generator can have run to its end in the meantime.
        if self.closed.load() {
            return Self::send_when_closed(jen, vm);
        }
        let value = if self.frame_opt().is_some_and(|f| f.lasti() > 0) {
            Some(vm.ctx.none())
        } else {
            None
        };
        let result = self.run_claimed(&claim, vm, |f| f.resume(value, vm));
        self.maybe_close(&result, &claim);
        drop(claim);
        self.finalize_send_result(result, jen, vm)
    }

    fn send_when_closed(jen: &PyObject, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        if jen.class().is(vm.ctx.types.coroutine_type) {
            Err(vm.new_runtime_error("cannot reuse already awaited coroutine"))
        } else {
            Ok(PyIterReturn::StopIteration(None))
        }
    }

    fn throw_when_closed(
        jen: &PyObject,
        exc_type: PyObjectRef,
        exc_val: PyObjectRef,
        exc_tb: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyIterReturn> {
        if jen.class().is(vm.ctx.types.coroutine_type) {
            Err(vm.new_runtime_error("cannot reuse already awaited coroutine"))
        } else {
            Err(vm.normalize_exception(exc_type, exc_val, exc_tb)?)
        }
    }

    pub fn send(
        &self,
        jen: &PyObject,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyIterReturn> {
        if self.closed.load() {
            return Self::send_when_closed(jen, vm);
        }
        let claim = self.claim(jen, vm)?;
        // The generator can have run to its end in the meantime.
        if self.closed.load() {
            return Self::send_when_closed(jen, vm);
        }
        let value = if self.frame_opt().is_some_and(|f| f.lasti() > 0) {
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
            return Self::throw_when_closed(jen, exc_type, exc_val, exc_tb, vm);
        }
        // Validate exception type before entering generator context.
        // Invalid types propagate to caller without closing the generator.
        crate::exceptions::ExceptionCtor::try_from_object(vm, exc_type.clone())?;
        let claim = self.claim(jen, vm)?;
        // The generator can have run to its end in the meantime. Normalizing
        // runs the exception's constructor, so let the claim go first.
        if self.closed.load() {
            drop(claim);
            return Self::throw_when_closed(jen, exc_type, exc_val, exc_tb, vm);
        }
        let result = self.run_claimed(&claim, vm, |f| f.gen_throw(vm, exc_type, exc_val, exc_tb));
        self.maybe_close(&result, &claim);
        drop(claim);
        self.finalize_send_result(result, jen, vm)
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
        // FRAME_CREATED: mark finished and clear the iframe.
        if self.frame_opt().is_none_or(|f| f.lasti() == 0) {
            self.closed.store(true);
            self.clear_except_code();
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
        if !matches!(&result, Ok(ExecutionResult::Yield(_))) {
            self.closed.store(true);
            self.clear_except_code();
        }
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
        !self.closed.load()
            && !self.running.load()
            && self.frame_opt().is_some_and(|f| f.lasti() > 0)
    }

    pub fn running(&self) -> bool {
        self.running.load()
    }

    pub fn closed(&self) -> bool {
        self.closed.load()
    }

    pub fn frame(&self) -> FrameObjectRef {
        self.frame_opt().expect("generator frame")
    }

    pub fn frame_opt(&self) -> Option<FrameObjectRef> {
        self.frame.try_to_owned(Ordering::Acquire)
    }

    pub fn code(&self) -> PyRef<PyCode> {
        self.code.clone()
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

fn iframe_origin_lineno(frame: &InterpreterFrame) -> usize {
    if frame.get_lasti() == 0 {
        return frame.code().first_line_number.map_or(1, |n| n.get());
    }
    let prev = frame.prev_line.get();
    if prev > 0 {
        prev as usize
    } else {
        frame.code().first_line_number.map_or(1, |n| n.get())
    }
}

/// Capture the current call stack as a coroutine `cr_origin` tuple.
///
/// Each entry is `(filename, lineno, funcname)`, innermost first. Depth 0
/// disables tracking and returns `None`.
pub(crate) fn compute_cr_origin(vm: &VirtualMachine) -> Option<PyTupleRef> {
    let depth = crate::vm::thread::COROUTINE_ORIGIN_TRACKING_DEPTH.get();
    if depth == 0 {
        return None;
    }
    let mut items = Vec::new();
    let mut iframe = crate::frame::current_thread_iframe();
    while !iframe.is_null() && items.len() < depth as usize {
        // SAFETY: TLS iframe chain entries stay alive while this thread runs.
        let frame = unsafe { &*iframe };
        let code = frame.code();
        items.push(
            vm.ctx
                .new_tuple(vec![
                    code.source_path().to_owned().into(),
                    vm.ctx.new_int(iframe_origin_lineno(frame)).into(),
                    code.obj_name.to_owned().into(),
                ])
                .into(),
        );
        iframe = frame.previous();
    }
    if items.is_empty() {
        None
    } else {
        Some(vm.ctx.new_tuple(items))
    }
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

pub(crate) fn unraisable_while_closing(
    jen: &PyObject,
    coro: &Coro,
    e: crate::builtins::PyBaseExceptionRef,
    vm: &VirtualMachine,
) {
    // Explicit close() leaves the traceback to the caller frame.
    // Finalize has no caller frame, so attach the generator site here.
    if e.__traceback__().is_none()
        && let Some(frame) = coro.frame_opt()
    {
        let lasti = frame.lasti().saturating_mul(2);
        let lineno = rustpython_compiler_core::OneIndexed::new(frame.f_lineno().max(1))
            .unwrap_or(rustpython_compiler_core::OneIndexed::MIN);
        let tb = PyTraceback::new(None, frame, lasti, lineno);
        e.set_traceback_typed(Some(tb.into_ref(&vm.ctx)));
    }
    let msg = jen
        .repr(vm)
        .ok()
        .map(|r| format!("Exception ignored while closing generator {r}"));
    vm.run_unraisable(e, msg, vm.ctx.none());
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
