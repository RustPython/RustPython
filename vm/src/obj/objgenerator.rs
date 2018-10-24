/*
 * The mythical generator.
 */

use super::super::frame::{ExecutionResult, Frame};
use super::super::pyobject::{
    AttributeProtocol, PyContext, PyFuncArgs, PyObject, PyObjectKind, PyObjectRef, PyResult,
    TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objtype;

pub fn init(context: &PyContext) {
    let ref generator_type = context.generator_type;
    generator_type.set_attr("__iter__", context.new_rustfunc(generator_iter));
    generator_type.set_attr("__next__", context.new_rustfunc(generator_next));
    generator_type.set_attr("send", context.new_rustfunc(generator_send));
}

pub fn new_generator(vm: &mut VirtualMachine, frame: Frame) -> PyResult {
    let g = PyObject::new(
        PyObjectKind::Generator { frame: frame },
        vm.ctx.generator_type.clone(),
    );
    Ok(g)
}

fn generator_iter(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(o, Some(vm.ctx.generator_type()))]);
    Ok(o.clone())
}

fn generator_next(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(o, Some(vm.ctx.generator_type()))]);
    let value = vm.get_none();
    send(vm, o, &value)
}

fn generator_send(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(o, Some(vm.ctx.generator_type())), (value, None)]
    );
    send(vm, o, value)
}

fn send(vm: &mut VirtualMachine, gen: &PyObjectRef, value: &PyObjectRef) -> PyResult {
    if let PyObjectKind::Generator { ref mut frame } = gen.borrow_mut().kind {
        frame.push_value(value.clone());
        match frame.run_frame(vm) {
            Ok(ExecutionResult::Yield(value)) => Ok(value),
            Ok(ExecutionResult::Return(_value)) => {
                // Stop iteration!
                let stop_iteration = vm.ctx.exceptions.stop_iteration.clone();
                Err(vm.new_exception(stop_iteration, "End of generator".to_string()))
            }
            Err(err) => Err(err),
        }
    } else {
        panic!("Cannot extract frame from non-generator");
    }
}
