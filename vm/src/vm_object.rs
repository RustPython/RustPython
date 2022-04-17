use crate::{
    builtins::{PyBaseExceptionRef, PyList, PyStr},
    function::{FuncArgs, IntoFuncArgs},
    vm::VirtualMachine,
    AsPyObject, PyMethod, PyObject, PyObjectRef, PyResult, PyValue, TypeProtocol,
};

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

/// PyObject support
impl VirtualMachine {
    #[track_caller]
    #[cold]
    fn _py_panic_failed(&self, exc: PyBaseExceptionRef, msg: &str) -> ! {
        #[cfg(not(all(target_arch = "wasm32", not(target_os = "wasi"))))]
        {
            let show_backtrace = std::env::var_os("RUST_BACKTRACE").map_or(false, |v| &v != "0");
            let after = if show_backtrace {
                self.print_exception(exc);
                "exception backtrace above"
            } else {
                "run with RUST_BACKTRACE=1 to see Python backtrace"
            };
            panic!("{}; {}", msg, after)
        }
        #[cfg(all(target_arch = "wasm32", not(target_os = "wasi")))]
        {
            use wasm_bindgen::prelude::*;
            #[wasm_bindgen]
            extern "C" {
                #[wasm_bindgen(js_namespace = console)]
                fn error(s: &str);
            }
            let mut s = String::new();
            self.write_exception(&mut s, &exc).unwrap();
            error(&s);
            panic!("{}; exception backtrace above", msg)
        }
    }

    #[track_caller]
    pub fn unwrap_pyresult<T>(&self, result: PyResult<T>) -> T {
        match result {
            Ok(x) => x,
            Err(exc) => {
                self._py_panic_failed(exc, "called `vm.unwrap_pyresult()` on an `Err` value")
            }
        }
    }
    #[track_caller]
    pub fn expect_pyresult<T>(&self, result: PyResult<T>, msg: &str) -> T {
        match result {
            Ok(x) => x,
            Err(exc) => self._py_panic_failed(exc, msg),
        }
    }

    /// Test whether a python object is `None`.
    pub fn is_none(&self, obj: &PyObject) -> bool {
        obj.is(&self.ctx.none)
    }
    pub fn option_if_none(&self, obj: PyObjectRef) -> Option<PyObjectRef> {
        if self.is_none(&obj) {
            None
        } else {
            Some(obj)
        }
    }
    pub fn unwrap_or_none(&self, obj: Option<PyObjectRef>) -> PyObjectRef {
        obj.unwrap_or_else(|| self.ctx.none())
    }

    pub fn call_get_descriptor_specific(
        &self,
        descr: PyObjectRef,
        obj: Option<PyObjectRef>,
        cls: Option<PyObjectRef>,
    ) -> Result<PyResult, PyObjectRef> {
        let descr_get = descr.class().mro_find_map(|cls| cls.slots.descr_get.load());
        match descr_get {
            Some(descr_get) => Ok(descr_get(descr, obj, cls, self)),
            None => Err(descr),
        }
    }

    pub fn call_get_descriptor(
        &self,
        descr: PyObjectRef,
        obj: PyObjectRef,
    ) -> Result<PyResult, PyObjectRef> {
        let cls = obj.clone_class().into();
        self.call_get_descriptor_specific(descr, Some(obj), Some(cls))
    }

    pub fn call_if_get_descriptor(&self, attr: PyObjectRef, obj: PyObjectRef) -> PyResult {
        self.call_get_descriptor(attr, obj).unwrap_or_else(Ok)
    }

    #[inline]
    pub fn call_method<T>(&self, obj: &PyObject, method_name: &str, args: T) -> PyResult
    where
        T: IntoFuncArgs,
    {
        flame_guard!(format!("call_method({:?})", method_name));

        PyMethod::get(
            obj.to_owned(),
            PyStr::from(method_name).into_ref(self),
            self,
        )?
        .invoke(args, self)
    }

    pub fn dir(&self, obj: Option<PyObjectRef>) -> PyResult<PyList> {
        let seq = match obj {
            Some(obj) => self
                .get_special_method(obj, "__dir__")?
                .map_err(|_obj| self.new_type_error("object does not provide __dir__".to_owned()))?
                .invoke((), self)?,
            None => self.call_method(self.current_locals()?.as_object(), "keys", ())?,
        };
        let items: Vec<_> = seq.try_to_value(self)?;
        let lst = PyList::from(items);
        lst.sort(Default::default(), self)?;
        Ok(lst)
    }

    #[inline]
    pub(crate) fn get_special_method(
        &self,
        obj: PyObjectRef,
        method: &str,
    ) -> PyResult<Result<PyMethod, PyObjectRef>> {
        PyMethod::get_special(obj, method, self)
    }

    /// NOT PUBLIC API
    #[doc(hidden)]
    pub fn call_special_method(
        &self,
        obj: PyObjectRef,
        method: &str,
        args: impl IntoFuncArgs,
    ) -> PyResult {
        self.get_special_method(obj, method)?
            .map_err(|_obj| self.new_attribute_error(method.to_owned()))?
            .invoke(args, self)
    }

    fn _invoke(&self, callable: &PyObject, args: FuncArgs) -> PyResult {
        vm_trace!("Invoke: {:?} {:?}", callable, args);
        let slot_call = callable.class().mro_find_map(|cls| cls.slots.call.load());
        match slot_call {
            Some(slot_call) => {
                self.trace_event(TraceEvent::Call)?;
                let result = slot_call(callable, args, self);
                self.trace_event(TraceEvent::Return)?;
                result
            }
            None => Err(self.new_type_error(format!(
                "'{}' object is not callable",
                callable.class().name()
            ))),
        }
    }

    #[inline(always)]
    pub fn invoke<O, A>(&self, func: &O, args: A) -> PyResult
    where
        O: AsRef<PyObject>,
        A: IntoFuncArgs,
    {
        self._invoke(func.as_ref(), args.into_args(self))
    }

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
