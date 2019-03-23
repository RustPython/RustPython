/*
 * The mythical generator.
 */

use crate::frame::{ExecutionResult, Frame};
use crate::function::PyFuncArgs;
use crate::pyobject::{PyContext, PyObjectRef, PyRef, PyResult, PyValue, TypeProtocol};
use crate::vm::VirtualMachine;

#[derive(Debug)]
pub struct PyGenerator {
    frame: PyObjectRef,
}
type PyGeneratorRef = PyRef<PyGenerator>;

impl PyValue for PyGenerator {
    fn class(vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.generator_type()
    }
}

pub fn init(context: &PyContext) {
    let generator_type = &context.generator_type;
    context.set_attr(
        &generator_type,
        "__iter__",
        context.new_rustfunc(generator_iter),
    );
    context.set_attr(
        &generator_type,
        "__next__",
        context.new_rustfunc(generator_next),
    );
    context.set_attr(
        &generator_type,
        "send",
        context.new_rustfunc(generator_send),
    );
}

pub fn new_generator(frame: PyObjectRef, vm: &VirtualMachine) -> PyGeneratorRef {
    PyGenerator { frame }.into_ref(vm)
}

fn generator_iter(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(o, Some(vm.ctx.generator_type()))]);
    Ok(o.clone())
}

fn generator_next(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(o, Some(vm.ctx.generator_type()))]);
    let value = vm.get_none();
    send(vm, o, &value)
}

fn generator_send(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(o, Some(vm.ctx.generator_type())), (value, None)]
    );
    send(vm, o, value)
}

fn send(vm: &VirtualMachine, gen: &PyObjectRef, value: &PyObjectRef) -> PyResult {
    if let Some(PyGenerator { ref frame }) = gen.payload() {
        if let Some(frame) = frame.payload::<Frame>() {
            frame.push_value(value.clone());
        } else {
            panic!("Generator frame isn't a frame.");
        }

        match vm.run_frame(frame.clone())? {
            ExecutionResult::Yield(value) => Ok(value),
            ExecutionResult::Return(_value) => {
                // Stop iteration!
                let stop_iteration = vm.ctx.exceptions.stop_iteration.clone();
                Err(vm.new_exception(stop_iteration, "End of generator".to_string()))
            }
        }
    } else {
        panic!("Cannot extract frame from non-generator");
    }
}
