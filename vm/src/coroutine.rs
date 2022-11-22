use crate::{
    AsObject, PyObject, PyObjectRef, PyResult, VirtualMachine,
    builtins::{PyBaseExceptionRef, PyStrRef},
    common::lock::PyMutex,
    frame::{ExecutionResult, Frame, FrameRef},
    object::PyAtomicRef,
    protocol::PyIterReturn,
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
    frame: PyAtomicRef<Option<Frame>>,
    running: AtomicCell<bool>,
    // code
    // _weakreflist
    name: PyMutex<PyStrRef>,
    // qualname
    exception: PyMutex<Option<PyBaseExceptionRef>>, // exc_state
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
    pub fn new(frame: FrameRef, name: PyStrRef) -> Self {
        Coro {
            frame: PyAtomicRef::from(Some(frame)),
            running: AtomicCell::new(false),
            exception: PyMutex::default(),
            name: PyMutex::new(name),
        }
    }

    fn maybe_close(&self, res: &PyResult<ExecutionResult>) {
        match res {
            Ok(ExecutionResult::Return(_)) | Err(_) => drop(unsafe { self.frame.swap(None) }),
            Ok(ExecutionResult::Yield(_)) => {}
        }
    }

    fn run_with_context<F>(
        &self,
        jen: &PyObject,
        frame: FrameRef,
        vm: &VirtualMachine,
        func: F,
    ) -> PyResult<ExecutionResult>
    where
        F: FnOnce(FrameRef) -> PyResult<ExecutionResult>,
    {
        if self.running.compare_exchange(false, true).is_err() {
            return Err(vm.new_value_error(format!("{} already executing", gen_name(jen, vm))));
        }

        vm.push_exception(self.exception.lock().take());
        let result = vm.with_frame(frame, func);

        *self.exception.lock() = vm.pop_exception();

        self.running.store(false);
        result
    }

    pub fn send(
        &self,
        jen: &PyObject,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyIterReturn> {
        let Some(frame) = self.frame.deref() else {
            return Ok(PyIterReturn::StopIteration(None));
        };
        let frame = frame.to_owned();
        let value = if frame.lasti() > 0 {
            Some(value)
        } else if !vm.is_none(&value) {
            return Err(vm.new_type_error(format!(
                "can't send non-None value to a just-started {}",
                gen_name(jen, vm),
            )));
        } else {
            None
        };
        let result = self.run_with_context(jen, frame, vm, |f| f.resume(value, vm));
        self.maybe_close(&result);
        match result {
            Ok(exec_res) => Ok(exec_res.into_iter_return(vm)),
            Err(e) => {
                if e.fast_isinstance(vm.ctx.exceptions.stop_iteration) {
                    let err =
                        vm.new_runtime_error(format!("{} raised StopIteration", gen_name(jen, vm)));
                    err.set_cause(Some(e));
                    Err(err)
                } else if jen.class().is(vm.ctx.types.async_generator)
                    && e.fast_isinstance(vm.ctx.exceptions.stop_async_iteration)
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
        jen: &PyObject,
        exc_type: PyObjectRef,
        exc_val: PyObjectRef,
        exc_tb: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyIterReturn> {
        let Some(frame) = self.frame.deref() else {
            return Err(vm.normalize_exception(exc_type, exc_val, exc_tb)?);
        };
        let result = self.run_with_context(jen, frame.to_owned(), vm, |f| {
            f.gen_throw(vm, exc_type, exc_val, exc_tb)
        });
        self.maybe_close(&result);
        Ok(result?.into_iter_return(vm))
    }

    pub fn close(&self, jen: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
        let Some(frame) = (unsafe { self.frame.swap(None) }) else {
            return Ok(());
        };
        let result = self.run_with_context(jen, frame, vm, |f| {
            f.gen_throw(
                vm,
                vm.ctx.exceptions.generator_exit.to_owned().into(),
                vm.ctx.none(),
                vm.ctx.none(),
            )
        });
        match result {
            Ok(ExecutionResult::Yield(_)) => {
                Err(vm.new_runtime_error(format!("{} ignored GeneratorExit", gen_name(jen, vm))))
            }
            Err(e) if !is_gen_exit(&e, vm) => Err(e),
            _ => Ok(()),
        }
    }

    pub fn set_close(&self) {
        drop(unsafe { self.frame.swap(None) });
    }

    pub fn running(&self) -> bool {
        self.running.load()
    }
    pub fn closed(&self) -> bool {
        self.frame.deref().is_none()
    }
    pub fn frame(&self) -> Option<FrameRef> {
        self.frame.deref().map(|x| x.to_owned())
    }
    pub fn name(&self) -> PyStrRef {
        self.name.lock().clone()
    }
    pub fn set_name(&self, name: PyStrRef) {
        *self.name.lock() = name;
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

pub fn is_gen_exit(exc: &PyBaseExceptionRef, vm: &VirtualMachine) -> bool {
    exc.fast_isinstance(vm.ctx.exceptions.generator_exit)
}
