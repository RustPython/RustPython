use super::{objiter, objtype};
use crate::exceptions::{self, PyBaseExceptionRef};
use crate::frame::{ExecutionResult, FrameRef};
use crate::pyobject::{PyObjectRef, PyResult};
use crate::vm::VirtualMachine;

use std::cell::Cell;

#[derive(Debug)]
pub struct Coro {
    frame: FrameRef,
    closed: Cell<bool>,
    running: Cell<bool>,
}

impl Coro {
    pub fn new(frame: FrameRef) -> Self {
        Coro {
            frame,
            closed: Cell::new(false),
            running: Cell::new(false),
        }
    }

    fn maybe_close(&self, res: &PyResult<ExecutionResult>) {
        match res {
            Ok(ExecutionResult::Return(_)) | Err(_) => self.closed.set(true),
            Ok(ExecutionResult::Yield(_)) => {}
        }
    }

    pub fn send(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if self.closed.get() {
            return Err(objiter::new_stop_iteration(vm));
        }

        self.frame.push_value(value.clone());
        self.running.set(true);
        let result = vm.run_frame(self.frame.clone());
        self.running.set(false);
        self.maybe_close(&result);
        result?.into_result(vm)
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
        self.running.set(true);
        let result = self.frame.gen_throw(vm, exc_type, exc_val, exc_tb);
        self.running.set(false);
        self.maybe_close(&result);
        vm.frames.borrow_mut().pop();
        result?.into_result(vm)
    }

    pub fn close(&self, vm: &VirtualMachine) -> PyResult<()> {
        if self.closed.get() {
            return Ok(());
        }
        vm.frames.borrow_mut().push(self.frame.clone());
        self.running.set(true);
        let result = self.frame.gen_throw(
            vm,
            vm.ctx.exceptions.generator_exit.clone().into_object(),
            vm.get_none(),
            vm.get_none(),
        );
        self.running.set(false);
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

    pub fn closed(&self) -> bool {
        self.closed.get()
    }
    pub fn running(&self) -> bool {
        self.running.get()
    }
    pub fn frame(&self) -> FrameRef {
        self.frame.clone()
    }
}

pub fn is_gen_exit(exc: &PyBaseExceptionRef, vm: &VirtualMachine) -> bool {
    objtype::isinstance(exc, &vm.ctx.exceptions.generator_exit)
}
