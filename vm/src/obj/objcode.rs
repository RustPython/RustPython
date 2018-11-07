/*! Infamous code object. The python class `code`

*/

use super::super::bytecode;
use super::super::pyobject::{
    AttributeProtocol, PyContext, PyFuncArgs, PyObjectKind, PyObjectRef, PyResult, TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objtype;

pub fn init(context: &PyContext) {
    let ref code_type = context.code_type;
    code_type.set_attr("__repr__", context.new_rustfunc(code_repr));
}

/// Extract rust bytecode object from a python code object.
pub fn copy_code(code_obj: &PyObjectRef) -> bytecode::CodeObject {
    let code_obj = code_obj.borrow();
    if let PyObjectKind::Code { ref code } = code_obj.kind {
        code.clone()
    } else {
        panic!("Must be code obj");
    }
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
    let line = format!(", line 1");

    let repr = format!("<code object at .. {}{}>", file, line);
    Ok(vm.new_str(repr))
}
