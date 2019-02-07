/*! Infamous code object. The python class `code`

*/

use super::super::bytecode;
use super::super::pyobject::{
    PyContext, PyFuncArgs, PyObjectPayload, PyObjectRef, PyResult, TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objtype;

pub fn init(context: &PyContext) {
    let code_type = &context.code_type;
    context.set_attr(code_type, "__new__", context.new_rustfunc(code_new));
    context.set_attr(code_type, "__repr__", context.new_rustfunc(code_repr));
    context.set_attr(
        code_type,
        "co_argcount",
        context.new_member_descriptor(code_co_argcount),
    );
}

/// Extract rust bytecode object from a python code object.
pub fn copy_code(code_obj: &PyObjectRef) -> bytecode::CodeObject {
    let code_obj = code_obj.borrow();
    if let PyObjectPayload::Code { ref code } = code_obj.payload {
        code.clone()
    } else {
        panic!("Must be code obj");
    }
}

fn code_new(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(_cls, None)]);
    Err(vm.new_type_error("Cannot directly create code object".to_string()))
}

fn code_repr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(o, Some(vm.ctx.code_type()))]);

    // Fetch actual code:
    let code = copy_code(o);

    let file = if let Some(source_path) = code.source_path {
        format!(", file {}", source_path)
    } else {
        String::new()
    };

    // TODO: fetch proper line info from code object
    let line = ", line 1".to_string();

    let repr = format!("<code object at .. {}{}>", file, line);
    Ok(vm.new_str(repr))
}

fn get_value(obj: &PyObjectRef) -> bytecode::CodeObject {
    if let PyObjectPayload::Code { code } = &obj.borrow().payload {
        code.clone()
    } else {
        panic!("Inner error getting code {:?}", obj)
    }
}

fn code_co_argcount(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [
            (zelf, Some(vm.ctx.code_type())),
            (_cls, Some(vm.ctx.type_type()))
        ]
    );
    let code_obj = get_value(zelf);
    Ok(vm.ctx.new_int(code_obj.arg_names.len()))
}
