use crate::function::PyFuncArgs;
use crate::pyobject::{PyContext, PyEllipsisRef, PyResult};
use crate::vm::VirtualMachine;

pub fn init(context: &PyContext) {
    extend_class!(context, &context.ellipsis_type, {
        "__new__" => context.new_rustfunc(ellipsis_new),
        "__repr__" => context.new_rustfunc(ellipsis_repr),
        "__reduce__" => context.new_rustfunc(ellipsis_reduce),
    });
}

fn ellipsis_new(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(_cls, None)]);
    Ok(vm.ctx.ellipsis())
}

fn ellipsis_repr(_self: PyEllipsisRef, _vm: &VirtualMachine) -> String {
    "Ellipsis".to_string()
}

fn ellipsis_reduce(_self: PyEllipsisRef, _vm: &VirtualMachine) -> String {
    "Ellipsis".to_string()
}
