/*
 * The mythical generator.
 */

use super::objiter::new_stop_iteration;
use super::objtype::{isinstance, PyClassRef};
use crate::frame::{ExecutionResult, FrameRef};
use crate::function::OptionalArg;
use crate::pyobject::{PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

use std::cell::Cell;

pub type PyGeneratorRef = PyRef<PyGenerator>;

#[pyclass(name = "generator")]
#[derive(Debug)]
pub struct PyGenerator {
    frame: FrameRef,
    closed: Cell<bool>,
}

impl PyValue for PyGenerator {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.generator_type()
    }
}

#[pyimpl]
impl PyGenerator {
    pub fn new(frame: FrameRef, vm: &VirtualMachine) -> PyGeneratorRef {
        PyGenerator {
            frame,
            closed: Cell::new(false),
        }
        .into_ref(vm)
    }

    fn maybe_close(&self, res: &PyResult<ExecutionResult>) {
        match res {
            Ok(ExecutionResult::Return(_)) | Err(_) => self.closed.set(true),
            Ok(ExecutionResult::Yield(_)) => {}
        }
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyGeneratorRef, _vm: &VirtualMachine) -> PyGeneratorRef {
        zelf
    }

    #[pymethod(name = "__next__")]
    fn next(&self, vm: &VirtualMachine) -> PyResult {
        self.send(vm.get_none(), vm)
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
}

pub fn init(ctx: &PyContext) {
    PyGenerator::extend_class(ctx, &ctx.types.generator_type);
}
