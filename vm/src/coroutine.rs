use crate::builtins::{PyStrRef, PyTypeRef};
use crate::exceptions::{self, PyBaseExceptionRef};
use crate::frame::{ExecutionResult, FrameRef};
use crate::pyobject::{PyObjectRef, PyResult, TypeProtocol};
use crate::vm::VirtualMachine;

use crate::common::lock::PyMutex;
use crossbeam_utils::atomic::AtomicCell;

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Variant {
    Gen,
    Coroutine,
    AsyncGen,
}
impl Variant {
    fn exec_result(self, res: ExecutionResult, vm: &VirtualMachine) -> PyResult {
        res.into_result(self == Self::AsyncGen, vm)
    }
    fn name(self) -> &'static str {
        match self {
            Self::Gen => "generator",
            Self::Coroutine => "coroutine",
            Self::AsyncGen => "async generator",
        }
    }
    fn stop_iteration(self, vm: &VirtualMachine) -> PyTypeRef {
        match self {
            Self::AsyncGen => vm.ctx.exceptions.stop_async_iteration.clone(),
            _ => vm.ctx.exceptions.stop_iteration.clone(),
        }
    }
}

#[derive(Debug)]
pub struct Coro {
    frame: FrameRef,
    pub closed: AtomicCell<bool>,
    running: AtomicCell<bool>,
    exceptions: PyMutex<Vec<PyBaseExceptionRef>>,
    started: AtomicCell<bool>,
    variant: Variant,
    name: PyMutex<PyStrRef>,
}

impl Coro {
    pub fn new(frame: FrameRef, variant: Variant, name: PyStrRef) -> Self {
        Coro {
            frame,
            closed: AtomicCell::new(false),
            running: AtomicCell::new(false),
            exceptions: PyMutex::new(vec![]),
            started: AtomicCell::new(false),
            variant,
            name: PyMutex::new(name),
        }
    }

    fn maybe_close(&self, res: &PyResult<ExecutionResult>) {
        match res {
            Ok(ExecutionResult::Return(_)) | Err(_) => self.closed.store(true),
            Ok(ExecutionResult::Yield(_)) => {}
        }
    }

    fn run_with_context<F>(&self, vm: &VirtualMachine, func: F) -> PyResult<ExecutionResult>
    where
        F: FnOnce(FrameRef) -> PyResult<ExecutionResult>,
    {
        self.running.store(true);
        let curr_exception_stack_len = vm.exceptions.borrow().len();
        vm.exceptions
            .borrow_mut()
            .append(&mut self.exceptions.lock());
        let result = vm.with_frame(self.frame.clone(), func);
        std::mem::swap(
            &mut *self.exceptions.lock(),
            &mut vm
                .exceptions
                .borrow_mut()
                .split_off(curr_exception_stack_len),
        );
        self.running.store(false);
        self.started.store(true);
        result
    }

    pub fn send(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if self.closed.load() {
            return Err(vm.new_exception_empty(self.variant.stop_iteration(vm)));
        }
        if !self.started.load() && !vm.is_none(&value) {
            return Err(vm.new_type_error(format!(
                "can't send non-None value to a just-started {}",
                self.variant.name()
            )));
        }
        let result = self.run_with_context(vm, |f| f.resume(value, vm));
        self.maybe_close(&result);
        match result {
            Ok(exec_res) => self.variant.exec_result(exec_res, vm),
            Err(e) => {
                if e.isinstance(&vm.ctx.exceptions.stop_iteration) {
                    let err = vm
                        .new_runtime_error(format!("{} raised StopIteration", self.variant.name()));
                    err.set_cause(Some(e));
                    Err(err)
                } else if self.variant == Variant::AsyncGen
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
        exc_type: PyObjectRef,
        exc_val: PyObjectRef,
        exc_tb: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult {
        if self.closed.load() {
            return Err(exceptions::normalize(exc_type, exc_val, exc_tb, vm)?);
        }
        let result = self.run_with_context(vm, |f| f.gen_throw(vm, exc_type, exc_val, exc_tb));
        self.maybe_close(&result);
        self.variant.exec_result(result?, vm)
    }

    pub fn close(&self, vm: &VirtualMachine) -> PyResult<()> {
        if self.closed.load() {
            return Ok(());
        }
        let result = self.run_with_context(vm, |f| {
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
                Err(vm.new_runtime_error(format!("{} ignored GeneratorExit", self.variant.name())))
            }
            Err(e) if !is_gen_exit(&e, vm) => Err(e),
            _ => Ok(()),
        }
    }

    pub fn started(&self) -> bool {
        self.started.load()
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
    pub fn repr(&self, id: usize) -> String {
        format!(
            "<{} object {} at {:#x}>",
            self.variant.name(),
            self.name.lock(),
            id
        )
    }
}

pub fn is_gen_exit(exc: &PyBaseExceptionRef, vm: &VirtualMachine) -> bool {
    exc.isinstance(&vm.ctx.exceptions.generator_exit)
}
