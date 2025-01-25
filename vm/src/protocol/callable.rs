use crate::{
    function::{FuncArgs, IntoFuncArgs},
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
        let call = obj.class().mro_find_map(|cls| cls.slots.call.load())?;
        Some(PyCallable { obj, call })
    }

    pub fn invoke(&self, args: impl IntoFuncArgs, vm: &VirtualMachine) -> PyResult {
        let args = args.into_args(vm);
        vm.trace_event(TraceEvent::Call)?;
        let result = (self.call)(self.obj, args, vm);
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
            let res = trace_func.call(args.clone(), self);
            self.use_tracing.set(true);
            if res.is_err() {
                *self.trace_func.borrow_mut() = self.ctx.none();
            }
        }

        if !self.is_none(&profile_func) {
            self.use_tracing.set(false);
            let res = profile_func.call(args, self);
            self.use_tracing.set(true);
            if res.is_err() {
                *self.profile_func.borrow_mut() = self.ctx.none();
            }
        }
        Ok(())
    }
}
