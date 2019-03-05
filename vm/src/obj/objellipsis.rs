use crate::pyobject::{PyContext, PyFuncArgs, PyResult, TypeProtocol};
use crate::vm::VirtualMachine;

pub fn init(context: &PyContext) {
    let ellipsis_type = &context.ellipsis_type;
    context.set_attr(ellipsis_type, "__new__", context.new_rustfunc(ellipsis_new));
    context.set_attr(
        ellipsis_type,
        "__repr__",
        context.new_rustfunc(ellipsis_repr),
    );
}

fn ellipsis_new(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(_cls, None)]);
    Ok(vm.ctx.ellipsis())
}

fn ellipsis_repr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(_cls, None)]);
    Ok(vm.new_str("Ellipsis".to_string()))
}
