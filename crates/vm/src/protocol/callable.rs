use crate::{
    builtins::{PyBoundMethod, PyFunction},
    function::{FuncArgs, IntoFuncArgs},
    types::{GenericMethod, PyTypeFlags, VectorCallFunc},
    {PyObject, PyObjectRef, PyResult, VirtualMachine},
};

impl PyObject {
    #[inline]
    #[must_use]
    pub fn to_callable(&self) -> Option<PyCallable<'_>> {
        PyCallable::new(self)
    }

    #[inline]
    #[must_use]
    pub fn is_callable(&self) -> bool {
        self.to_callable().is_some()
    }

    /// PyObject_Call*Arg* series
    #[inline]
    pub fn call(&self, args: impl IntoFuncArgs, vm: &VirtualMachine) -> PyResult {
        let args = args.into_args(vm);
        self.call_with_args(args, vm)
    }

    /// PyObject_Call
    pub fn call_with_args(&self, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let Some(callable) = self.to_callable() else {
            return Err(vm.new_type_error(format!(
                "'{}' object is not callable",
                self.class().slot_name()
            )));
        };
        vm_trace!("Invoke: {:?} {:?}", callable, args);
        callable.invoke(args, vm)
    }

    /// Vectorcall: call with owned positional args + optional kwnames.
    /// Falls back to FuncArgs-based call if no vectorcall slot.
    #[inline]
    pub fn vectorcall(
        &self,
        args: Vec<PyObjectRef>,
        nargs: usize,
        kwnames: Option<&[PyObjectRef]>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let Some(callable) = self.to_callable() else {
            return Err(vm.new_type_error(format!(
                "'{}' object is not callable",
                self.class().slot_name()
            )));
        };
        callable.invoke_vectorcall(args, nargs, kwnames, vm)
    }

    /// PyVectorcall_Call: invoke retained raw support without automatic eligibility
    /// or a tp_call fallback.
    pub fn vectorcall_call(
        &self,
        args: Vec<PyObjectRef>,
        nargs: usize,
        kwnames: Option<&[PyObjectRef]>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let vc = self.raw_vectorcall_func().ok_or_else(|| {
            vm.new_type_error(format!(
                "'{}' object does not support vectorcall",
                self.class().slot_name()
            ))
        })?;
        self.invoke_vectorcall_func(vc, args, nargs, kwnames, vm)
    }

    #[inline]
    pub(crate) fn vectorcall_func(&self) -> Option<VectorCallFunc> {
        if self
            .class()
            .slots()
            .flags
            .has_feature(PyTypeFlags::HAVE_VECTORCALL)
        {
            self.raw_vectorcall_func()
        } else {
            None
        }
    }

    #[inline]
    fn raw_vectorcall_func(&self) -> Option<VectorCallFunc> {
        let slots = self.class().slots();
        slots.vectorcall.load().or_else(|| {
            slots
                .vectorcall_resolver
                .load()
                .and_then(|resolve| resolve(self))
        })
    }

    #[inline]
    fn invoke_vectorcall_func(
        &self,
        vc: VectorCallFunc,
        args: Vec<PyObjectRef>,
        nargs: usize,
        kwnames: Option<&[PyObjectRef]>,
        vm: &VirtualMachine,
    ) -> PyResult {
        if !vm.use_tracing.get() {
            return vc(self, args, nargs, kwnames, vm);
        }
        let is_python_callable = self.downcast_ref::<PyFunction>().is_some()
            || self.downcast_ref::<PyBoundMethod>().is_some();
        if is_python_callable {
            vc(self, args, nargs, kwnames, vm)
        } else {
            let callable = self.to_owned();
            vm.trace_event(TraceEvent::CCall, Some(callable.clone()))?;
            let result = vc(self, args, nargs, kwnames, vm);
            if result.is_ok() {
                vm.trace_event(TraceEvent::CReturn, Some(callable))?;
            } else {
                let _ = vm.trace_event(TraceEvent::CException, Some(callable));
            }
            result
        }
    }
}

#[derive(Debug)]
pub struct PyCallable<'a> {
    pub obj: &'a PyObject,
    pub call: GenericMethod,
    pub vectorcall: Option<VectorCallFunc>,
}

impl<'a> PyCallable<'a> {
    pub fn new(obj: &'a PyObject) -> Option<Self> {
        let slots = obj.class().slots();
        let call = slots.call.load()?;
        let vectorcall = obj.vectorcall_func();
        Some(PyCallable {
            obj,
            call,
            vectorcall,
        })
    }

    pub fn invoke(&self, args: impl IntoFuncArgs, vm: &VirtualMachine) -> PyResult {
        let args = args.into_args(vm);
        if self.vectorcall_revoked() {
            // A retained wrapper must use current tp_call, not its cached snapshot.
            return self.obj.call_with_args(args, vm);
        }
        if let Some(vc) = self.vectorcall
            && args.kwargs.is_empty()
        {
            let nargs = args.args.len();
            return self
                .obj
                .invoke_vectorcall_func(vc, args.args, nargs, None, vm);
        }
        if !vm.use_tracing.get() {
            return (self.call)(self.obj, args, vm);
        }
        // Python functions get 'call'/'return' events from with_frame().
        // Bound methods delegate to the inner callable, which fires its own events.
        // All other callables (built-in functions, etc.) get 'c_call'/'c_return'/'c_exception'.
        let is_python_callable = self.obj.downcast_ref::<PyFunction>().is_some()
            || self.obj.downcast_ref::<PyBoundMethod>().is_some();
        if is_python_callable {
            (self.call)(self.obj, args, vm)
        } else {
            let callable = self.obj.to_owned();
            vm.trace_event(TraceEvent::CCall, Some(callable.clone()))?;
            let result = (self.call)(self.obj, args, vm);
            if result.is_ok() {
                vm.trace_event(TraceEvent::CReturn, Some(callable))?;
            } else {
                let _ = vm.trace_event(TraceEvent::CException, Some(callable));
            }
            result
        }
    }

    /// Use automatic vectorcall if available, else fall back to FuncArgs.
    #[inline]
    pub fn invoke_vectorcall(
        &self,
        args: Vec<PyObjectRef>,
        nargs: usize,
        kwnames: Option<&[PyObjectRef]>,
        vm: &VirtualMachine,
    ) -> PyResult {
        if self.vectorcall_revoked() {
            return self.obj.vectorcall(args, nargs, kwnames, vm);
        }
        if let Some(vc) = self.vectorcall {
            self.obj
                .invoke_vectorcall_func(vc, args, nargs, kwnames, vm)
        } else {
            // Fallback: convert owned Vec to FuncArgs (move, no clone)
            let func_args = FuncArgs::from_vectorcall_owned(args, nargs, kwnames);
            self.invoke(func_args, vm)
        }
    }

    #[inline]
    fn vectorcall_revoked(&self) -> bool {
        self.vectorcall.is_some()
            && !self
                .obj
                .class()
                .slots()
                .flags
                .has_feature(PyTypeFlags::HAVE_VECTORCALL)
    }
}

/// Trace events for sys.settrace and sys.setprofile.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum TraceEvent {
    Call,
    Return,
    Exception,
    Line,
    Opcode,
    CCall,
    CReturn,
    CException,
}

// Existing callers retain their combined hook behavior. Virtual monitoring
// dispatch can resolve each participant separately at its invocation turn.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum TraceParticipant {
    ExistingBoth,
    Trace,
    Profile,
}

impl TraceEvent {
    /// Whether sys.settrace receives this event.
    #[must_use]
    const fn is_trace_event(self) -> bool {
        matches!(
            self,
            Self::Call | Self::Return | Self::Exception | Self::Line | Self::Opcode
        )
    }

    /// Whether sys.setprofile receives this event.
    /// In legacy_tracing.c, profile callbacks are only registered for
    /// PY_RETURN, PY_UNWIND, C_CALL, C_RETURN, C_RAISE.
    #[must_use]
    const fn is_profile_event(self) -> bool {
        matches!(
            self,
            Self::Call | Self::Return | Self::CCall | Self::CReturn | Self::CException
        )
    }

    /// Whether this event is dispatched only when f_trace_opcodes is set.
    #[must_use]
    pub(crate) const fn is_opcode_event(self) -> bool {
        matches!(self, Self::Opcode)
    }

    /// Default `what_event` for this legacy event.
    #[must_use]
    const fn default_what(self) -> crate::stdlib::sys::monitoring::MonitoringEvent {
        use crate::stdlib::sys::monitoring::MonitoringEvent as Ev;
        match self {
            Self::Call => Ev::PyStart,
            Self::Return => Ev::PyReturn,
            Self::Exception => Ev::Raise,
            Self::Line => Ev::Line,
            Self::Opcode => Ev::Instruction,
            Self::CCall => Ev::Call,
            Self::CReturn => Ev::CReturn,
            Self::CException => Ev::CRaise,
        }
    }
}

impl core::fmt::Display for TraceEvent {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Call => write!(f, "call"),
            Self::Return => write!(f, "return"),
            Self::Exception => write!(f, "exception"),
            Self::Line => write!(f, "line"),
            Self::Opcode => write!(f, "opcode"),
            Self::CCall => write!(f, "c_call"),
            Self::CReturn => write!(f, "c_return"),
            Self::CException => write!(f, "c_exception"),
        }
    }
}

impl VirtualMachine {
    /// Call registered trace function.
    ///
    /// Returns the trace function's return value:
    /// - `Some(obj)` if the trace function returned a non-None value
    /// - `None` if it returned Python None or no trace function was active
    ///
    /// In CPython's trace protocol:
    /// - For 'call' events: the return value determines the per-frame `f_trace`
    /// - For 'line'/'return' events: the return value can update `f_trace`
    #[inline]
    pub(crate) fn trace_event(
        &self,
        event: TraceEvent,
        arg: Option<PyObjectRef>,
    ) -> PyResult<Option<PyObjectRef>> {
        self.trace_event_what(event, event.default_what(), arg)
    }

    /// Like [`Self::trace_event`], but records `what` as `tstate->what_event`
    /// for the duration of the callback (so `f_lineno` assignment can tell
    /// a 'line' event from a 'call'/'return'/'exception').
    #[inline]
    pub(crate) fn trace_event_what(
        &self,
        event: TraceEvent,
        what: crate::stdlib::sys::monitoring::MonitoringEvent,
        arg: Option<PyObjectRef>,
    ) -> PyResult<Option<PyObjectRef>> {
        self.trace_participant_what(event, what, arg, TraceParticipant::ExistingBoth)
    }

    pub(crate) fn trace_participant_what(
        &self,
        event: TraceEvent,
        what: crate::stdlib::sys::monitoring::MonitoringEvent,
        arg: Option<PyObjectRef>,
        participant: TraceParticipant,
    ) -> PyResult<Option<PyObjectRef>> {
        if self.use_tracing.get() && !self.tracing_is_suppressed() {
            let old = self.what_event.replace(Some(what));
            let result = self._trace_event_inner(event, arg, participant);
            self.what_event.set(old);
            result
        } else {
            Ok(None)
        }
    }
    fn _trace_event_inner(
        &self,
        event: TraceEvent,
        arg: Option<PyObjectRef>,
        participant: TraceParticipant,
    ) -> PyResult<Option<PyObjectRef>> {
        if self.is_none(&self.trace_func.borrow()) && self.is_none(&self.profile_func.borrow()) {
            return Ok(None);
        }
        let trace_func = if participant != TraceParticipant::Profile {
            self.trace_func.borrow().to_owned()
        } else {
            self.ctx.none()
        };
        let profile_func = if participant != TraceParticipant::Trace {
            self.profile_func.borrow().to_owned()
        } else {
            self.ctx.none()
        };
        let is_trace_event = participant != TraceParticipant::Profile && event.is_trace_event();
        let is_profile_event = participant != TraceParticipant::Trace && event.is_profile_event();
        let is_opcode_event = event.is_opcode_event();

        let Some(frame_ref) = crate::frame::current_complete_frame_for_trace(self) else {
            return Ok(None);
        };

        // Opcode events are only dispatched when f_trace_opcodes is set.
        if is_opcode_event
            && !frame_ref
                .iframe()
                .cold_opt()
                .is_some_and(|c| *c.trace_opcodes.lock())
        {
            return Ok(None);
        }

        // trace_trampoline: CALL uses the global callback; every other
        // event uses the per-frame f_trace (and is a no-op if that is unset).
        let callback = if event == TraceEvent::Call {
            if self.is_none(&trace_func) {
                None
            } else {
                Some(trace_func)
            }
        } else {
            frame_ref
                .iframe()
                .cold_opt()
                .and_then(|c| c.trace.lock().clone())
        };

        let frame: PyObjectRef = frame_ref.to_owned().into();
        let event_str: PyObjectRef = self.ctx.new_str(event.to_string()).into();
        let args = vec![frame, event_str, arg.unwrap_or_else(|| self.ctx.none())];

        let mut trace_result = None;

        // temporarily disable tracing, during the call to the
        // tracing function itself.
        if is_trace_event && let Some(callback) = callback {
            self.use_tracing.set(false);
            self.enter_tracing();
            let res = callback.call(args.clone(), self);
            self.leave_tracing();
            self.use_tracing.set(true);
            match res {
                Ok(result) => {
                    if !self.is_none(&result) {
                        *frame_ref.iframe().cold().trace.lock() = Some(result.clone());
                        trace_result = Some(result);
                    }
                }
                Err(e) => {
                    // trace_trampoline: disable the global tracer and clear
                    // this frame's f_trace, then propagate.
                    *self.trace_func.borrow_mut() = self.ctx.none();
                    *frame_ref.iframe().cold().trace.lock() = None;
                    let profile_is_none = self.is_none(&self.profile_func.borrow());
                    self.use_tracing.set(!profile_is_none);
                    return Err(e);
                }
            }
        }

        if is_profile_event && !self.is_none(&profile_func) {
            self.use_tracing.set(false);
            self.enter_tracing();
            let res = profile_func.call(args, self);
            self.leave_tracing();
            self.use_tracing.set(true);
            if res.is_err() {
                *self.profile_func.borrow_mut() = self.ctx.none();
            }
        }
        Ok(trace_result)
    }
}
