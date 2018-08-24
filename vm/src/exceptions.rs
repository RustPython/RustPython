use super::pyobject::{AttributeProtocol, PyContext, PyFuncArgs, PyResult};
use super::vm::VirtualMachine;

fn exception_init(vm: &mut VirtualMachine, _args: PyFuncArgs) -> PyResult {
    Ok(vm.get_none())
}

pub fn init(context: &PyContext) {
    let ref base_exception_type = context.base_exception_type;
    base_exception_type.set_attr("__init__", context.new_rustfunc(exception_init));

    // TODO: create a whole exception hierarchy somehow?
}
