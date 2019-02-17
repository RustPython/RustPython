/*! Infamous code object. The python class `code`

*/

use super::super::bytecode;
use super::super::pyobject::{
    IdProtocol, PyContext, PyFuncArgs, PyObjectPayload, PyObjectRef, PyResult, TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objtype;

pub fn init(context: &PyContext) {
    let code_type = &context.code_type;
    context.set_attr(code_type, "__new__", context.new_rustfunc(code_new));
    context.set_attr(code_type, "__repr__", context.new_rustfunc(code_repr));

    for (name, f) in &[
        (
            "co_argcount",
            code_co_argcount as fn(&mut VirtualMachine, PyFuncArgs) -> PyResult,
        ),
        ("co_consts", code_co_consts),
        ("co_filename", code_co_filename),
        ("co_firstlineno", code_co_firstlineno),
        ("co_kwonlyargcount", code_co_kwonlyargcount),
        ("co_name", code_co_name),
    ] {
        context.set_attr(code_type, name, context.new_member_descriptor(f))
    }
}

pub fn get_value(obj: &PyObjectRef) -> bytecode::CodeObject {
    if let PyObjectPayload::Code { code } = &obj.borrow().payload {
        code.clone()
    } else {
        panic!("Inner error getting code {:?}", obj)
    }
}

fn code_new(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(_cls, None)]);
    Err(vm.new_type_error("Cannot directly create code object".to_string()))
}

fn code_repr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(o, Some(vm.ctx.code_type()))]);

    let code = get_value(o);
    let repr = format!(
        "<code object {} at 0x{:x} file {:?}, line {}>",
        code.obj_name,
        o.get_id(),
        code.source_path,
        code.first_line_number
    );
    Ok(vm.new_str(repr))
}

fn member_code_obj(
    vm: &mut VirtualMachine,
    args: PyFuncArgs,
) -> Result<bytecode::CodeObject, PyObjectRef> {
    arg_check!(
        vm,
        args,
        required = [
            (zelf, Some(vm.ctx.code_type())),
            (_cls, Some(vm.ctx.type_type()))
        ]
    );
    Ok(get_value(zelf))
}

fn code_co_argcount(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    let code_obj = member_code_obj(vm, args)?;
    Ok(vm.ctx.new_int(code_obj.arg_names.len()))
}

fn code_co_filename(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    let code_obj = member_code_obj(vm, args)?;
    let source_path = code_obj.source_path;
    Ok(vm.new_str(source_path))
}

fn code_co_firstlineno(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    let code_obj = member_code_obj(vm, args)?;
    Ok(vm.ctx.new_int(code_obj.first_line_number))
}

fn code_co_kwonlyargcount(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    let code_obj = member_code_obj(vm, args)?;
    Ok(vm.ctx.new_int(code_obj.kwonlyarg_names.len()))
}

fn code_co_consts(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    let code_obj = member_code_obj(vm, args)?;
    let consts = code_obj
        .get_constants()
        .map(|x| vm.ctx.unwrap_constant(x))
        .collect();
    Ok(vm.ctx.new_tuple(consts))
}

fn code_co_name(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    let code_obj = member_code_obj(vm, args)?;
    Ok(vm.new_str(code_obj.obj_name))
}
