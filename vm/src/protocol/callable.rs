use crate::{
    function::IntoFuncArgs,
    types::GenericMethod,
    {AsObject, PyObject, PyResult, VirtualMachine},
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
}

pub struct PyCallable<'a> {
    pub obj: &'a PyObject,
    pub call: GenericMethod,
}

impl<'a> PyCallable<'a> {
    pub fn new(obj: &'a PyObject) -> Option<Self> {
        let call = obj.class().mro_find_map(|cls| cls.slots.call.load())?;
        Some(PyCallable { obj, call })
    }

    pub fn invoke(&self, args: impl IntoFuncArgs, vm: &VirtualMachine) -> PyResult {
        vm.trace_event(TraceEvent::Call)?;
        let result = (self.call)(self.obj, args.into_args(vm), vm);
        vm.trace_event(TraceEvent::Return)?;
        result
    }
}

/// Trace events for sys.settrace and sys.setprofile.
enum TraceEvent {
    Call,
    Return,
}

impl std::fmt::Display for TraceEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        use TraceEvent::*;
        match self {
            Call => write!(f, "call"),
            Return => write!(f, "return"),
        }
    }
}

impl VirtualMachine {
    /// Call registered trace function.
    #[inline]
    fn trace_event(&self, event: TraceEvent) -> PyResult<()> {
        if self.use_tracing.get() {
            self._trace_event_inner(event)
        } else {
            Ok(())
        }
    }
    fn _trace_event_inner(&self, event: TraceEvent) -> PyResult<()> {
        let trace_func = self.trace_func.borrow().to_owned();
        let profile_func = self.profile_func.borrow().to_owned();
        if self.is_none(&trace_func) && self.is_none(&profile_func) {
            return Ok(());
        }

        let frame_ref = self.current_frame();
        if frame_ref.is_none() {
            return Ok(());
        }

        let frame = frame_ref.unwrap().as_object().to_owned();
        let event = self.ctx.new_str(event.to_string()).into();
        let args = vec![frame, event, self.ctx.none()];

        // temporarily disable tracing, during the call to the
        // tracing function itself.
        if !self.is_none(&trace_func) {
            self.use_tracing.set(false);
            let res = self.invoke(&trace_func, args.clone());
            self.use_tracing.set(true);
            res?;
        }

        if !self.is_none(&profile_func) {
            self.use_tracing.set(false);
            let res = self.invoke(&profile_func, args);
            self.use_tracing.set(true);
            res?;
        }
        Ok(())
    }
}
