/*
 * The mythical generator.
 */

use super::objiter::new_stop_iteration;
use super::objtype::{isinstance, issubclass, PyClassRef};
use crate::frame::{ExecutionResult, FrameRef};
use crate::function::OptionalArg;
use crate::pyobject::{
    PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue, TryFromObject,
};
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
        exc_type: PyClassRef,
        exc_val: OptionalArg,
        exc_tb: OptionalArg,
        vm: &VirtualMachine,
    ) -> PyResult {
        if self.closed.get() {
            return Err(TryFromObject::try_from_object(
                vm,
                vm.invoke(exc_type.as_object(), vec![])?,
            )?);
        }
        // TODO what should we do with the other parameters? CPython normalises them with
        //      PyErr_NormalizeException, do we want to do the same.
        if !issubclass(&exc_type, &vm.ctx.exceptions.base_exception_type) {
            return Err(vm.new_type_error("Can't throw non exception".to_string()));
        }
        vm.frames.borrow_mut().push(self.frame.clone());
        let result = self.frame.gen_throw(
            vm,
            exc_type,
            exc_val.unwrap_or(vm.get_none()),
            exc_tb.unwrap_or(vm.get_none()),
        );
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
            vm.ctx.exceptions.generator_exit.clone(),
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
