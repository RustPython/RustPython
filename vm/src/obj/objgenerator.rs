/*
 * The mythical generator.
 */

use crate::frame::{ExecutionResult, FrameRef};
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{PyContext, PyObjectRef, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

pub type PyGeneratorRef = PyRef<PyGenerator>;

#[derive(Debug)]
pub struct PyGenerator {
    frame: FrameRef,
}

impl PyValue for PyGenerator {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.generator_type()
    }
}

impl PyGeneratorRef {
    pub fn new(frame: FrameRef, vm: &VirtualMachine) -> PyGeneratorRef {
        PyGenerator { frame }.into_ref(vm)
    }

    fn iter(self, _vm: &VirtualMachine) -> PyGeneratorRef {
        self
    }

    fn next(self, vm: &VirtualMachine) -> PyResult {
        self.send(vm.get_none(), vm)
    }

    fn send(self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.frame.push_value(value.clone());

        match vm.run_frame(self.frame.clone())? {
            ExecutionResult::Yield(value) => Ok(value),
            ExecutionResult::Return(_value) => {
                // Stop iteration!
                let stop_iteration = vm.ctx.exceptions.stop_iteration.clone();
                Err(vm.new_exception(stop_iteration, "End of generator".to_string()))
            }
        }
    }
}

pub fn init(context: &PyContext) {
    let generator_type = &context.generator_type;
    extend_class!(context, generator_type, {
        "__iter__" => context.new_rustfunc(PyGeneratorRef::iter),
        "__next__" => context.new_rustfunc(PyGeneratorRef::next),
        "send" => context.new_rustfunc(PyGeneratorRef::send)
    });
}
