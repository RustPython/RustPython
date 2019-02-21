use super::super::obj::objcode;
use super::super::obj::objtype;
use super::super::pyobject::{PyContext, PyFuncArgs, PyObjectRef, PyResult, TypeProtocol};
use super::super::vm::VirtualMachine;

fn dis_disassemble(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(co, Some(vm.ctx.code_type()))]);

    let code = objcode::get_value(co);
    print!("{}", code);
    Ok(vm.get_none())
}

pub fn mk_module(ctx: &PyContext) -> PyObjectRef {
    let py_mod = ctx.new_module("dis", ctx.new_scope(None));
    ctx.set_attr(&py_mod, "disassemble", ctx.new_rustfunc(dis_disassemble));
    py_mod
}
