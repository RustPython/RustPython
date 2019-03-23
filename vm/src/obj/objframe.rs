/*! The python `frame` type.

*/

use crate::frame::Frame;
use crate::function::PyFuncArgs;
use crate::pyobject::{PyContext, PyObjectRef, PyResult, TypeProtocol};
use crate::vm::VirtualMachine;

pub fn init(context: &PyContext) {
    let frame_type = &context.frame_type;
    extend_class!(context, frame_type, {
        "__new__" => context.new_rustfunc(frame_new),
        "__repr__" => context.new_rustfunc(frame_repr),
        "f_locals" => context.new_property(frame_flocals),
        "f_code" => context.new_property(frame_fcode)
    });
}

fn frame_new(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(_cls, None)]);
    Err(vm.new_type_error("Cannot directly create frame object".to_string()))
}

fn frame_repr(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(_frame, Some(vm.ctx.frame_type()))]);
    let repr = "<frame object at .. >".to_string();
    Ok(vm.new_str(repr))
}

fn frame_flocals(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(frame, Some(vm.ctx.frame_type()))]);
    let frame = get_value(frame);
    Ok(frame.scope.get_locals().clone())
}

fn frame_fcode(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(frame, Some(vm.ctx.frame_type()))]);
    Ok(vm.ctx.new_code_object(get_value(frame).code.clone()))
}

pub fn get_value(obj: &PyObjectRef) -> &Frame {
    &obj.payload::<Frame>().unwrap()
}
