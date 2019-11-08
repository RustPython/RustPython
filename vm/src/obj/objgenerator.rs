/*
 * The mythical generator.
 */

use super::objtype::{isinstance, issubclass, PyClassRef};
use crate::frame::{ExecutionResult, FrameRef};
use crate::function::OptionalArg;
use crate::pyobject::{PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

pub type PyGeneratorRef = PyRef<PyGenerator>;

#[pyclass(name = "generator")]
#[derive(Debug)]
pub struct PyGenerator {
    frame: FrameRef,
}

impl PyValue for PyGenerator {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.generator_type()
    }
}

#[pyimpl]
impl PyGenerator {
    pub fn new(frame: FrameRef, vm: &VirtualMachine) -> PyGeneratorRef {
        PyGenerator { frame }.into_ref(vm)
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
        self.frame.push_value(value.clone());

        vm.run_frame(self.frame.clone())?.into_result(vm)
    }

    #[pymethod]
    fn throw(
        &self,
        exc_type: PyClassRef,
        exc_val: OptionalArg,
        exc_tb: OptionalArg,
        vm: &VirtualMachine,
    ) -> PyResult {
        // TODO what should we do with the other parameters? CPython normalises them with
        //      PyErr_NormalizeException, do we want to do the same.
        if !issubclass(&exc_type, &vm.ctx.exceptions.base_exception_type) {
            return Err(vm.new_type_error("Can't throw non exception".to_string()));
        }
        vm.frames.borrow_mut().push(self.frame.clone());
        let result = self
            .frame
            .gen_throw(
                vm,
                exc_type,
                exc_val.unwrap_or(vm.get_none()),
                exc_tb.unwrap_or(vm.get_none()),
            )
            .and_then(|res| res.into_result(vm));
        vm.frames.borrow_mut().pop();
        result
    }

    #[pymethod]
    fn close(&self, vm: &VirtualMachine) -> PyResult<()> {
        vm.frames.borrow_mut().push(self.frame.clone());
        let result = self.frame.gen_throw(
            vm,
            vm.ctx.exceptions.generator_exit.clone(),
            vm.get_none(),
            vm.get_none(),
        );
        vm.frames.borrow_mut().pop();
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
