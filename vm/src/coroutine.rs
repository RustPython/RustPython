use crate::{
    builtins::{PyBaseExceptionRef, PyStrRef},
    common::lock::PyMutex,
    exceptions,
    frame::{ExecutionResult, FrameRef},
    protocol::PyIterReturn,
    IdProtocol, PyObjectRef, PyResult, TypeProtocol, VirtualMachine,
};
use crossbeam_utils::atomic::AtomicCell;

impl ExecutionResult {
    /// Turn an ExecutionResult into a PyResult that would be returned from a generator or coroutine
    fn into_iter_return(self, vm: &VirtualMachine) -> PyIterReturn {
        match self {
            ExecutionResult::Yield(value) => PyIterReturn::Return(value),
            ExecutionResult::Return(value) => {
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
    // qualname
    exception: PyMutex<Option<PyBaseExceptionRef>>, // exc_state
}

fn gen_name(gen: &PyObjectRef, vm: &VirtualMachine) -> &'static str {
    let typ = gen.class();
    if typ.is(&vm.ctx.types.coroutine_type) {
        "coroutine"
    } else if typ.is(&vm.ctx.types.async_generator) {
        "async generator"
    } else {
        "generator"
    }
}

impl Coro {
    pub fn new(frame: FrameRef, name: PyStrRef) -> Self {
        Coro {
            frame,
            closed: AtomicCell::new(false),
            running: AtomicCell::new(false),
            exception: PyMutex::default(),
            name: PyMutex::new(name),
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
        gen: &PyObjectRef,
        vm: &VirtualMachine,
        func: F,
    ) -> PyResult<ExecutionResult>
    where
        F: FnOnce(FrameRef) -> PyResult<ExecutionResult>,
    {
        if self.running.compare_exchange(false, true).is_err() {
            return Err(vm.new_value_error(format!("{} already executing", gen_name(gen, vm))));
        }

        vm.push_exception(self.exception.lock().take());

        let result = vm.with_frame(self.frame.clone(), func);

        *self.exception.lock() = vm.pop_exception();

        self.running.store(false);
        result
    }

    pub fn send(
        &self,
        gen: &PyObjectRef,
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
                gen_name(gen, vm),
            )));
        } else {
            None
        };
        let result = self.run_with_context(gen, vm, |f| f.resume(value, vm));
        self.maybe_close(&result);
        match result {
            Ok(exec_res) => Ok(exec_res.into_iter_return(vm)),
            Err(e) => {
                if e.isinstance(&vm.ctx.exceptions.stop_iteration) {
                    let err =
                        vm.new_runtime_error(format!("{} raised StopIteration", gen_name(gen, vm)));
                    err.set_cause(Some(e));
                    Err(err)
                } else if gen.class().is(&vm.ctx.types.async_generator)
                    && e.isinstance(&vm.ctx.exceptions.stop_async_iteration)
                {
                    let err = vm
                        .new_runtime_error("async generator raised StopAsyncIteration".to_owned());
                    err.set_cause(Some(e));
                    Err(err)
                } else {
                    Err(e)
                }
            }
        }
    }
    pub fn throw(
        &self,
        gen: &PyObjectRef,
        exc_type: PyObjectRef,
        exc_val: PyObjectRef,
        exc_tb: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyIterReturn> {
        if self.closed.load() {
            return Err(exceptions::normalize(exc_type, exc_val, exc_tb, vm)?);
        }
        let result = self.run_with_context(gen, vm, |f| f.gen_throw(vm, exc_type, exc_val, exc_tb));
        self.maybe_close(&result);
        Ok(result?.into_iter_return(vm))
    }

    pub fn close(&self, gen: &PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        if self.closed.load() {
            return Ok(());
        }
        let result = self.run_with_context(gen, vm, |f| {
            f.gen_throw(
                vm,
                vm.ctx.exceptions.generator_exit.clone().into_object(),
                vm.ctx.none(),
                vm.ctx.none(),
            )
        });
        self.closed.store(true);
        match result {
            Ok(ExecutionResult::Yield(_)) => {
                Err(vm.new_runtime_error(format!("{} ignored GeneratorExit", gen_name(gen, vm))))
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
    pub fn repr(&self, gen: &PyObjectRef, id: usize, vm: &VirtualMachine) -> String {
        format!(
            "<{} object {} at {:#x}>",
            gen_name(gen, vm),
            self.name.lock(),
            id
        )
    }
}

pub fn is_gen_exit(exc: &PyBaseExceptionRef, vm: &VirtualMachine) -> bool {
    exc.isinstance(&vm.ctx.exceptions.generator_exit)
}
