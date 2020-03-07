use super::objtype;
use crate::exceptions::{self, PyBaseExceptionRef};
use crate::frame::{ExecutionResult, FrameRef};
use crate::pyobject::{PyObjectRef, PyResult};
use crate::vm::VirtualMachine;

use std::cell::{Cell, RefCell};

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
}

#[derive(Debug)]
pub struct Coro {
    frame: FrameRef,
    pub closed: Cell<bool>,
    running: Cell<bool>,
    exceptions: RefCell<Vec<PyBaseExceptionRef>>,
    started: Cell<bool>,
    variant: Variant,
}

impl Coro {
    pub fn new(frame: FrameRef, variant: Variant) -> Self {
        Coro {
            frame,
            closed: Cell::new(false),
            running: Cell::new(false),
            exceptions: RefCell::new(vec![]),
            started: Cell::new(false),
            variant,
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
            let cls = if let Variant::AsyncGen = self.variant {
                vm.ctx.exceptions.stop_async_iteration.clone()
            } else {
                vm.ctx.exceptions.stop_iteration.clone()
            };
            return Err(vm.new_exception_empty(cls));
        }
        if !self.started.get() && !vm.is_none(&value) {
            return Err(vm.new_type_error(format!(
                "can't send non-None value to a just-started {}",
                self.variant.name()
            )));
        }
        self.frame.push_value(value.clone());
        let result = self.run_with_context(|| vm.run_frame(self.frame.clone()), vm);
        self.maybe_close(&result);
        match result {
            Ok(exec_res) => self.variant.exec_result(exec_res, vm),
            Err(e) => {
                if objtype::isinstance(&e, &vm.ctx.exceptions.stop_iteration) {
                    let err = vm
                        .new_runtime_error(format!("{} raised StopIteration", self.variant.name()));
                    err.set_cause(Some(e));
                    Err(err)
                } else if self.variant == Variant::AsyncGen
                    && objtype::isinstance(&e, &vm.ctx.exceptions.stop_async_iteration)
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
        if self.closed.get() {
            return Err(exceptions::normalize(exc_type, exc_val, exc_tb, vm)?);
        }
        vm.frames.borrow_mut().push(self.frame.clone());
        let result =
            self.run_with_context(|| self.frame.gen_throw(vm, exc_type, exc_val, exc_tb), vm);
        self.maybe_close(&result);
        vm.frames.borrow_mut().pop();
        self.variant.exec_result(result?, vm)
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
                Err(vm.new_runtime_error(format!("{} ignored GeneratorExit", self.variant.name())))
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
    pub fn name(&self) -> String {
        self.frame.code.obj_name.clone()
    }
}

pub fn is_gen_exit(exc: &PyBaseExceptionRef, vm: &VirtualMachine) -> bool {
    objtype::isinstance(exc, &vm.ctx.exceptions.generator_exit)
}
