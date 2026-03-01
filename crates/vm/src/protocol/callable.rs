use crate::{
    builtins::{PyBoundMethod, PyFunction},
    function::{FuncArgs, IntoFuncArgs},
    types::GenericMethod,
    {PyObject, PyObjectRef, PyResult, VirtualMachine},
};

impl PyObject {
    #[inline]
    pub fn to_callable(&self) -> Option<PyCallable<'_>> {
        PyCallable::new(self)
    }

    #[inline]
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
            return Err(
                vm.new_type_error(format!("'{}' object is not callable", self.class().name()))
            );
        };
        vm_trace!("Invoke: {:?} {:?}", callable, args);
        callable.invoke(args, vm)
    }
}

#[derive(Debug)]
pub struct PyCallable<'a> {
    pub obj: &'a PyObject,
    pub call: GenericMethod,
}

impl<'a> PyCallable<'a> {
    pub fn new(obj: &'a PyObject) -> Option<Self> {
        let call = obj.class().slots.call.load()?;
        Some(PyCallable { obj, call })
    }

    pub fn invoke(&self, args: impl IntoFuncArgs, vm: &VirtualMachine) -> PyResult {
        let args = args.into_args(vm);
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
}

/// Trace events for sys.settrace and sys.setprofile.
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

impl TraceEvent {
    /// Whether sys.setprofile receives this event.
    /// In legacy_tracing.c, profile callbacks are only registered for
    /// PY_RETURN, PY_UNWIND, C_CALL, C_RETURN, C_RAISE.
    fn is_profile_event(&self) -> bool {
        matches!(
            self,
            Self::Call | Self::Return | Self::CCall | Self::CReturn | Self::CException
        )
    }

    /// Whether this event is dispatched only when f_trace_opcodes is set.
    pub(crate) fn is_opcode_event(&self) -> bool {
        matches!(self, Self::Opcode)
    }
}

impl core::fmt::Display for TraceEvent {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        use TraceEvent::*;
        match self {
            Call => write!(f, "call"),
            Return => write!(f, "return"),
            Exception => write!(f, "exception"),
            Line => write!(f, "line"),
            Opcode => write!(f, "opcode"),
            CCall => write!(f, "c_call"),
            CReturn => write!(f, "c_return"),
            CException => write!(f, "c_exception"),
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
        if self.use_tracing.get() {
            self._trace_event_inner(event, arg)
        } else {
            Ok(None)
        }
    }
    fn _trace_event_inner(
        &self,
        event: TraceEvent,
        arg: Option<PyObjectRef>,
    ) -> PyResult<Option<PyObjectRef>> {
        let trace_func = self.trace_func.borrow().to_owned();
        let profile_func = self.profile_func.borrow().to_owned();
        if self.is_none(&trace_func) && self.is_none(&profile_func) {
            return Ok(None);
        }

        let is_profile_event = event.is_profile_event();
        let is_opcode_event = event.is_opcode_event();

        let Some(frame_ref) = self.current_frame() else {
            return Ok(None);
        };

        // Opcode events are only dispatched when f_trace_opcodes is set.
        if is_opcode_event && !*frame_ref.trace_opcodes.lock() {
            return Ok(None);
        }

        let frame: PyObjectRef = frame_ref.into();
        let event = self.ctx.new_str(event.to_string()).into();
        let args = vec![frame, event, arg.unwrap_or_else(|| self.ctx.none())];

        let mut trace_result = None;

        // temporarily disable tracing, during the call to the
        // tracing function itself.
        if !self.is_none(&trace_func) {
            self.use_tracing.set(false);
            let res = trace_func.call(args.clone(), self);
            self.use_tracing.set(true);
            match res {
                Ok(result) => {
                    if !self.is_none(&result) {
                        trace_result = Some(result);
                    }
                }
                Err(e) => {
                    // trace_trampoline behavior: clear per-frame f_trace
                    // and propagate the error.
                    if let Some(frame_ref) = self.current_frame() {
                        *frame_ref.trace.lock() = self.ctx.none();
                    }
                    return Err(e);
                }
            }
        }

        if is_profile_event && !self.is_none(&profile_func) {
            self.use_tracing.set(false);
            let res = profile_func.call(args, self);
            self.use_tracing.set(true);
            if res.is_err() {
                *self.profile_func.borrow_mut() = self.ctx.none();
            }
        }
        Ok(trace_result)
    }
}
