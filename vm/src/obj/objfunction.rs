use super::super::pyobject::{AttributeProtocol, PyContext, PyFuncArgs, PyResult};
use super::super::vm::VirtualMachine;

pub fn init(context: &PyContext) {
    let ref function_type = context.function_type;
    function_type.set_attr("__get__", context.new_rustfunc(bind_method));

    let ref member_descriptor_type = context.member_descriptor_type;
    member_descriptor_type.set_attr("__get__", context.new_rustfunc(member_get));
}

fn bind_method(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    Ok(vm.new_bound_method(args.args[0].clone(), args.args[1].clone()))
}

fn member_get(vm: &mut VirtualMachine, mut args: PyFuncArgs) -> PyResult {
    match args.shift().get_attr("function") {
        Some(function) => vm.invoke(function, args),
        None => {
            let attribute_error = vm.context().exceptions.attribute_error.clone();
            Err(vm.new_exception(attribute_error, String::from("Attribute Error")))
        }
    }
}
