/*! The python `frame` type.

*/

use super::super::pyobject::{
    AttributeProtocol, PyContext, PyFuncArgs, PyObjectKind, PyObjectRef, PyResult, TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objtype;

pub fn init(context: &PyContext) {
    let ref frame_type = context.frame_type;
    frame_type.set_attr("__new__", context.new_rustfunc(frame_new));
    frame_type.set_attr("__repr__", context.new_rustfunc(frame_repr));
    frame_type.set_attr("f_locals", context.new_property(frame_flocals));
    frame_type.set_attr("f_code", context.new_property(frame_fcode));
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

fn frame_flocals(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(frame, Some(vm.ctx.frame_type()))]);
    if let PyObjectKind::Frame { ref frame } = frame.borrow().kind {
        let py_scope = frame.locals.clone();
        let py_scope = py_scope.borrow();

        if let PyObjectKind::Scope { scope } = &py_scope.kind {
            Ok(scope.locals.clone())
        } else {
            panic!("The scope isn't a scope!");
        }
    } else {
        panic!("Frame doesn't contain a frame: {:?}", frame);
    }
}

fn frame_fcode(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(frame, Some(vm.ctx.frame_type()))]);
    if let PyObjectKind::Frame { ref frame } = frame.borrow().kind {
        Ok(vm.ctx.new_code_object(frame.code.clone()))
    } else {
        panic!("Frame doesn't contain a frame: {:?}", frame);
    }
}
