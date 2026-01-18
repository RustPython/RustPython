use crate::{
    AsObject, Py, PyObject, PyObjectRef, PyResult, VirtualMachine,
    builtins::{PyBaseExceptionRef, PyStrRef},
    common::lock::PyMutex,
    exceptions::types::PyBaseException,
    frame::{ExecutionResult, FrameRef},
    function::OptionalArg,
    object::{Traverse, TraverseFn},
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
        }
    }
}

#[derive(Debug)]
pub struct Coro {
    frame: FrameRef,
    pub closed: AtomicCell<bool>, // TODO: https://github.com/RustPython/RustPython/pull/3183#discussion_r720560652
    running: AtomicCell<bool>,
    // code
    // _weakreflist
    name: PyMutex<PyStrRef>,
    qualname: PyMutex<PyStrRef>,
    exception: PyMutex<Option<PyBaseExceptionRef>>, // exc_state
}

unsafe impl Traverse for Coro {
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        self.frame.traverse(tracer_fn);
        self.name.traverse(tracer_fn);
        self.qualname.traverse(tracer_fn);
        self.exception.traverse(tracer_fn);
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
    pub fn new(frame: FrameRef, name: PyStrRef, qualname: PyStrRef) -> Self {
        Self {
            frame,
            closed: AtomicCell::new(false),
            running: AtomicCell::new(false),
            exception: PyMutex::default(),
            name: PyMutex::new(name),
            qualname: PyMutex::new(qualname),
        }
    }

    fn maybe_close(&self, res: &PyResult<ExecutionResult>) {
        match res {
            Ok(ExecutionResult::Return(_)) | Err(_) => self.closed.store(true),
            Ok(ExecutionResult::Yield(_)) => {}
        }
    }

    fn run_with_context<F>(
        &self,
        jen: &PyObject,
        vm: &VirtualMachine,
        func: F,
    ) -> PyResult<ExecutionResult>
    where
        F: FnOnce(FrameRef) -> PyResult<ExecutionResult>,
    {
        if self.running.compare_exchange(false, true).is_err() {
            return Err(vm.new_value_error(format!("{} already executing", gen_name(jen, vm))));
        }

        // swap exception state
        // Get generator's saved exception state from last yield
        let gen_exc = self.exception.lock().take();

        // Use a slot to capture generator's exception state before with_frame pops
        let exception_slot = &self.exception;

        // Run the generator frame
        // with_frame does push_exception(None) which creates a new exception context
        // The caller's exception remains in the chain via prev, so topmost_exception()
        // will find it if generator's exception is None
        let result = vm.with_frame(self.frame.clone(), |f| {
            // with_frame pushed None, creating: { exc: None, prev: caller's exc_info }
            // Pop None and push generator's exception instead
            // This maintains the chain: { exc: gen_exc, prev: caller's exc_info }
            vm.pop_exception();
            vm.push_exception(gen_exc);
            let result = func(f);
            // Save generator's exception state BEFORE with_frame pops
            // This is the generator's current exception context
            *exception_slot.lock() = vm.current_exception();
            result
        });

        self.running.store(false);
        result
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
        let result = self.run_with_context(jen, vm, |f| f.resume(value, vm));
        self.maybe_close(&result);
        match result {
            Ok(exec_res) => Ok(exec_res.into_iter_return(vm)),
            Err(e) => {
                if e.fast_isinstance(vm.ctx.exceptions.stop_iteration) {
                    let err =
                        vm.new_runtime_error(format!("{} raised StopIteration", gen_name(jen, vm)));
                    err.set___cause__(Some(e));
                    Err(err)
                } else if jen.class().is(vm.ctx.types.async_generator)
                    && e.fast_isinstance(vm.ctx.exceptions.stop_async_iteration)
                {
                    let err = vm.new_runtime_error("async generator raised StopAsyncIteration");
                    err.set___cause__(Some(e));
                    Err(err)
                } else {
                    Err(e)
                }
            }
        }
    }

    pub fn throw(
        &self,
        jen: &PyObject,
        exc_type: PyObjectRef,
        exc_val: PyObjectRef,
        exc_tb: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyIterReturn> {
        if self.closed.load() {
            return Err(vm.normalize_exception(exc_type, exc_val, exc_tb)?);
        }
        let result = self.run_with_context(jen, vm, |f| f.gen_throw(vm, exc_type, exc_val, exc_tb));
        self.maybe_close(&result);
        Ok(result?.into_iter_return(vm))
    }

    pub fn close(&self, jen: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
        if self.closed.load() {
            return Ok(());
        }
        // If generator hasn't started (FRAME_CREATED), just mark as closed
        if self.frame.lasti() == 0 {
            self.closed.store(true);
            return Ok(());
        }
        let result = self.run_with_context(jen, vm, |f| {
            f.gen_throw(
                vm,
                vm.ctx.exceptions.generator_exit.to_owned().into(),
                vm.ctx.none(),
                vm.ctx.none(),
            )
        });
        self.closed.store(true);
        match result {
            Ok(ExecutionResult::Yield(_)) => {
                Err(vm.new_runtime_error(format!("{} ignored GeneratorExit", gen_name(jen, vm))))
            }
            Err(e) if !is_gen_exit(&e, vm) => Err(e),
            _ => Ok(()),
        }
    }

    pub fn running(&self) -> bool {
        self.running.load()
    }

    pub fn closed(&self) -> bool {
        self.closed.load()
    }

    pub fn frame(&self) -> FrameRef {
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
        format!(
            "<{} object {} at {:#x}>",
            gen_name(jen, vm),
            self.name.lock(),
            id
        )
    }
}

pub fn is_gen_exit(exc: &Py<PyBaseException>, vm: &VirtualMachine) -> bool {
    exc.fast_isinstance(vm.ctx.exceptions.generator_exit)
}

/// Emit DeprecationWarning for the deprecated 3-argument throw() signature.
pub fn warn_deprecated_throw_signature(
    exc_val: &OptionalArg,
    exc_tb: &OptionalArg,
    vm: &VirtualMachine,
) -> PyResult<()> {
    if exc_val.is_present() || exc_tb.is_present() {
        crate::warn::warn(
            vm.ctx.new_str(
                "the (type, val, tb) signature of throw() is deprecated, \
                 use throw(val) instead",
            ),
            Some(vm.ctx.exceptions.deprecation_warning.to_owned()),
            1,
            None,
            vm,
        )?;
    }
    Ok(())
}
