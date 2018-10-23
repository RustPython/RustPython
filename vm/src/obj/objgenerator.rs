/*
 * The mythical generator.
 */

use super::super::frame::Frame;
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
    generator_type.set_attr("__send__", context.new_rustfunc(generator_send));
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
    send(vm, o)
}

fn generator_send(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(o, Some(vm.ctx.generator_type()))]);
    send(vm, o)
}

fn send(vm: &mut VirtualMachine, _gen: &PyObjectRef) -> PyResult {
    /*
       TODO
        if let PyObjectKind::Generator { frame } = &gen.borrow_mut().kind {
            vm.run_frame(frame)
        } else {
            panic!("Cannot extract frame from non-generator");
        }
    */
    Ok(vm.get_none())
}
