/*
 * The mythical generator.
 */

use crate::frame::{ExecutionResult, FrameRef};
use crate::obj::objtype::{isinstance, PyClassRef};
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
    fn send(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.frame.push_value(value.clone());

        let result = vm.run_frame(self.frame.clone())?;
        handle_execution_result(result, vm)
    }

    #[pymethod]
    fn throw(
        &self,
        _exc_type: PyObjectRef,
        exc_val: PyObjectRef,
        _exc_tb: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult {
        // TODO what should we do with the other parameters? CPython normalises them with
        //      PyErr_NormalizeException, do we want to do the same.
        if !isinstance(&exc_val, &vm.ctx.exceptions.base_exception_type) {
            return Err(vm.new_type_error("Can't throw non exception".to_string()));
        }
        let result = vm.frame_throw(self.frame.clone(), exc_val)?;
        handle_execution_result(result, vm)
    }
}

fn handle_execution_result(result: ExecutionResult, vm: &VirtualMachine) -> PyResult {
    match result {
        ExecutionResult::Yield(value) => Ok(value),
        ExecutionResult::Return(_value) => {
            // Stop iteration!
            let stop_iteration = vm.ctx.exceptions.stop_iteration.clone();
            Err(vm.new_exception(stop_iteration, "End of generator".to_string()))
        }
    }
}

pub fn init(ctx: &PyContext) {
    PyGenerator::extend_class(ctx, &ctx.generator_type);
}
