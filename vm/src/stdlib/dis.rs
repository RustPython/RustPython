use crate::obj::objcode;
use crate::pyobject::{PyContext, PyFuncArgs, PyObjectRef, PyResult, TypeProtocol};
use crate::vm::VirtualMachine;

fn dis_dis(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(obj, None)]);
    let code_name = vm.new_str("__code__".to_string());
    let code = match vm.get_attribute(obj.clone(), code_name) {
        Ok(co) => co,
        Err(..) => obj.clone(),
    };

    dis_disassemble(vm, PyFuncArgs::new(vec![code], vec![]))
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
