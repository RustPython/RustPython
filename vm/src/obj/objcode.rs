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
