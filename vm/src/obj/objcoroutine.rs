use super::objiter::new_stop_iteration;
use super::objtype::{isinstance, PyClassRef};
use crate::frame::{ExecutionResult, FrameRef};
use crate::function::OptionalArg;
use crate::pyobject::{PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

use std::cell::Cell;

pub type PyCoroutineRef = PyRef<PyCoroutine>;

#[pyclass(name = "coroutine")]
#[derive(Debug)]
pub struct PyCoroutine {
    frame: FrameRef,
    closed: Cell<bool>,
}

impl PyValue for PyCoroutine {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.types.coroutine_type.clone()
    }
}

#[pyimpl]
impl PyCoroutine {
    pub fn new(frame: FrameRef, vm: &VirtualMachine) -> PyCoroutineRef {
        PyCoroutine {
            frame,
            closed: Cell::new(false),
        }
        .into_ref(vm)
    }

    // TODO: deduplicate this code with objgenerator
    fn maybe_close(&self, res: &PyResult<ExecutionResult>) {
        match res {
            Ok(ExecutionResult::Return(_)) | Err(_) => self.closed.set(true),
            Ok(ExecutionResult::Yield(_)) => {}
        }
    }

    #[pymethod]
    pub(crate) fn send(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if self.closed.get() {
            return Err(new_stop_iteration(vm));
        }

        self.frame.push_value(value.clone());

        let result = vm.run_frame(self.frame.clone());
        self.maybe_close(&result);
        result?.into_result(vm)
    }

    #[pymethod]
    fn throw(
        &self,
        exc_type: PyObjectRef,
        exc_val: OptionalArg,
        exc_tb: OptionalArg,
        vm: &VirtualMachine,
    ) -> PyResult {
        let exc_val = exc_val.unwrap_or_else(|| vm.get_none());
        let exc_tb = exc_tb.unwrap_or_else(|| vm.get_none());
        if self.closed.get() {
            return Err(vm.normalize_exception(exc_type, exc_val, exc_tb)?);
        }
        vm.frames.borrow_mut().push(self.frame.clone());
        let result = self.frame.gen_throw(vm, exc_type, exc_val, exc_tb);
        self.maybe_close(&result);
        vm.frames.borrow_mut().pop();
        result?.into_result(vm)
    }

    #[pymethod]
    fn close(&self, vm: &VirtualMachine) -> PyResult<()> {
        if self.closed.get() {
            return Ok(());
        }
        vm.frames.borrow_mut().push(self.frame.clone());
        let result = self.frame.gen_throw(
            vm,
            vm.ctx.exceptions.generator_exit.clone().into_object(),
            vm.get_none(),
            vm.get_none(),
        );
        vm.frames.borrow_mut().pop();
        self.closed.set(true);
        match result {
            Ok(ExecutionResult::Yield(_)) => Err(vm.new_exception(
                vm.ctx.exceptions.runtime_error.clone(),
                "generator ignored GeneratorExit".to_string(),
            )),
            Err(e) => {
                if isinstance(&e, &vm.ctx.exceptions.generator_exit) {
                    Ok(())
                } else {
                    Err(e)
                }
            }
            _ => Ok(()),
        }
    }

    #[pymethod(name = "__await__")]
    fn r#await(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyCoroutineWrapper {
        PyCoroutineWrapper { coro: zelf }
    }
}

#[pyclass(name = "coroutine_wrapper")]
#[derive(Debug)]
pub struct PyCoroutineWrapper {
    coro: PyCoroutineRef,
}

impl PyValue for PyCoroutineWrapper {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.types.coroutine_wrapper_type.clone()
    }
}

#[pyimpl]
impl PyCoroutineWrapper {
    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyRef<Self> {
        zelf
    }

    #[pymethod(name = "__next__")]
    fn next(&self, vm: &VirtualMachine) -> PyResult {
        self.coro.send(vm.get_none(), vm)
    }

    #[pymethod]
    fn send(&self, val: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.coro.send(val, vm)
    }

    #[pymethod]
    fn throw(
        &self,
        exc_type: PyObjectRef,
        exc_val: OptionalArg,
        exc_tb: OptionalArg,
        vm: &VirtualMachine,
    ) -> PyResult {
        self.coro.throw(exc_type, exc_val, exc_tb, vm)
    }
}

pub fn init(ctx: &PyContext) {
    PyCoroutine::extend_class(ctx, &ctx.types.coroutine_type);
    PyCoroutineWrapper::extend_class(ctx, &ctx.types.coroutine_wrapper_type);
}
