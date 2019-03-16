use crate::function::PyFuncArgs;
use crate::obj::objcode;
use crate::pyobject::{PyContext, PyObjectRef, PyResult, TypeProtocol};
use crate::vm::VirtualMachine;

fn dis_dis(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(obj, None)]);

    // Method or function:
    let code_name = vm.new_str("__code__".to_string());
    if let Ok(co) = vm.get_attribute(obj.clone(), code_name) {
        return dis_disassemble(vm, PyFuncArgs::new(vec![co], vec![]));
    }

    dis_disassemble(vm, PyFuncArgs::new(vec![obj.clone()], vec![]))
}

fn dis_disassemble(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(co, Some(vm.ctx.code_type()))]);

    let code = objcode::get_value(co);
    print!("{}", code);
    Ok(vm.get_none())
}

pub fn make_module(ctx: &PyContext) -> PyObjectRef {
    py_module!(ctx, "dis", {
        "dis" => ctx.new_rustfunc(dis_dis),
        "disassemble" => ctx.new_rustfunc(dis_disassemble)
    })
}
