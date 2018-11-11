/*! The python `frame` type.

*/

use super::super::frame::Frame;
use super::super::pyobject::{
    AttributeProtocol, PyContext, PyFuncArgs, PyObjectKind, PyObjectRef, PyResult, TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objtype;

pub fn init(context: &PyContext) {
    let ref frame_type = context.frame_type;
    frame_type.set_attr("__new__", context.new_rustfunc(frame_new));
    frame_type.set_attr("__repr__", context.new_rustfunc(frame_repr));
    frame_type.set_attr("f_locals", context.new_property(frame_locals));
}

fn frame_new(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(_cls, None)]);
    Err(vm.new_type_error(format!("Cannot directly create frame object")))
}

fn frame_repr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(_frame, Some(vm.ctx.frame_type()))]);
    let repr = format!("<frame object at .. >");
    Ok(vm.new_str(repr))
}

fn frame_locals(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(frame, Some(vm.ctx.frame_type()))]);
    let frame = get_value(frame);
    let py_scope = frame.locals.clone();
    let py_scope = py_scope.borrow();

    if let PyObjectKind::Scope { scope } = &py_scope.kind {
        Ok(scope.locals.clone())
    } else {
        panic!("The scope isn't a scope!");
    }
}

pub fn get_value(obj: &PyObjectRef) -> Frame {
    if let PyObjectKind::Frame { frame } = &obj.borrow().kind {
        frame.clone()
    } else {
        panic!("Inner error getting int {:?}", obj);
    }
}
