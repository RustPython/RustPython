/*! The python `frame` type.

*/

use super::super::pyobject::{
    AttributeProtocol, PyContext, PyFuncArgs, PyObjectRef, PyResult, TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objtype;

pub fn init(context: &PyContext) {
    let ref frame_type = context.frame_type;
    frame_type.set_attr("__repr__", context.new_rustfunc(frame_repr));
}

fn frame_repr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(_frame, Some(vm.ctx.frame_type()))]);
    let repr = format!("<frame object at .. >");
    Ok(vm.new_str(repr))
}
