use super::{objiter, objtype};
use crate::exceptions::{self, PyBaseExceptionRef};
use crate::frame::{ExecutionResult, FrameRef};
use crate::pyobject::{PyObjectRef, PyResult};
use crate::vm::VirtualMachine;

use std::cell::{Cell, RefCell};

#[derive(Debug)]
pub struct Coro {
    frame: FrameRef,
    pub closed: Cell<bool>,
    running: Cell<bool>,
    exceptions: RefCell<Vec<PyBaseExceptionRef>>,
    started: Cell<bool>,
    async_iter: bool,
}

impl Coro {
    pub fn new(frame: FrameRef) -> Self {
        Coro {
            frame,
            closed: Cell::new(false),
            running: Cell::new(false),
            exceptions: RefCell::new(vec![]),
            started: Cell::new(false),
            async_iter: false,
        }
    }
    pub fn new_async(frame: FrameRef) -> Self {
        Coro {
            frame,
            closed: Cell::new(false),
            running: Cell::new(false),
            exceptions: RefCell::new(vec![]),
            started: Cell::new(false),
            async_iter: true,
        }
    }

    fn maybe_close(&self, res: &PyResult<ExecutionResult>) {
        match res {
            Ok(ExecutionResult::Return(_)) | Err(_) => self.closed.set(true),
            Ok(ExecutionResult::Yield(_)) => {}
        }
    }

    fn run_with_context<F>(&self, func: F, vm: &VirtualMachine) -> PyResult<ExecutionResult>
    where
        F: FnOnce() -> PyResult<ExecutionResult>,
    {
        self.running.set(true);
        let curr_exception_stack_len = vm.exceptions.borrow().len();
        vm.exceptions
            .borrow_mut()
            .append(&mut self.exceptions.borrow_mut());
        let result = func();
        self.exceptions.replace(
            vm.exceptions
                .borrow_mut()
                .split_off(curr_exception_stack_len),
        );
        self.running.set(false);
        self.started.set(true);
        result
    }

    pub fn send(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if self.closed.get() {
            return Err(objiter::new_stop_iteration(vm));
        }
        if !self.started.get() && !vm.is_none(&value) {
            return Err(vm.new_type_error(
                "can't send non-None value to a just-started coroutine".to_string(),
            ));
        }
        self.frame.push_value(value.clone());
        let result = self.run_with_context(|| vm.run_frame(self.frame.clone()), vm);
        self.maybe_close(&result);
        result?.into_result(self.async_iter, vm)
    }
    pub fn throw(
        &self,
        exc_type: PyObjectRef,
        exc_val: PyObjectRef,
        exc_tb: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult {
        if self.closed.get() {
            return Err(exceptions::normalize(exc_type, exc_val, exc_tb, vm)?);
        }
        vm.frames.borrow_mut().push(self.frame.clone());
        let result =
            self.run_with_context(|| self.frame.gen_throw(vm, exc_type, exc_val, exc_tb), vm);
        self.maybe_close(&result);
        vm.frames.borrow_mut().pop();
        result?.into_result(self.async_iter, vm)
    }

    pub fn close(&self, vm: &VirtualMachine) -> PyResult<()> {
        if self.closed.get() {
            return Ok(());
        }
        vm.frames.borrow_mut().push(self.frame.clone());
        let result = self.run_with_context(
            || {
                self.frame.gen_throw(
                    vm,
                    vm.ctx.exceptions.generator_exit.clone().into_object(),
                    vm.get_none(),
                    vm.get_none(),
                )
            },
            vm,
        );
        vm.frames.borrow_mut().pop();
        self.closed.set(true);
        match result {
            Ok(ExecutionResult::Yield(_)) => {
                Err(vm.new_runtime_error("generator ignored GeneratorExit".to_owned()))
            }
            Err(e) if !is_gen_exit(&e, vm) => Err(e),
            _ => Ok(()),
        }
    }

    pub fn started(&self) -> bool {
        self.started.get()
    }
    pub fn running(&self) -> bool {
        self.running.get()
    }
    pub fn closed(&self) -> bool {
        self.closed.get()
    }
    pub fn frame(&self) -> FrameRef {
        self.frame.clone()
    }
}

pub fn is_gen_exit(exc: &PyBaseExceptionRef, vm: &VirtualMachine) -> bool {
    objtype::isinstance(exc, &vm.ctx.exceptions.generator_exit)
}
