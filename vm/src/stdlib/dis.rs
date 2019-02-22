use crate::obj::objcode;
use crate::pyobject::{PyContext, PyFuncArgs, PyObjectRef, PyResult, TypeProtocol};
use crate::vm::VirtualMachine;

fn dis_dis(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    dis_disassemble(vm, args)
}

fn dis_disassemble(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(co, Some(vm.ctx.code_type()))]);

    let code = objcode::get_value(co);
    print!("{}", code);
    Ok(vm.get_none())
}

pub fn mk_module(ctx: &PyContext) -> PyObjectRef {
    py_module!(ctx, "dis", {
        "dis" => ctx.new_rustfunc(dis_dis),
        "disassemble" => ctx.new_rustfunc(dis_disassemble)
    })
}
