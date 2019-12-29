use crate::function::OptionalArg;
use crate::obj::{objiter, objtype};
use crate::pyobject::{PyObjectRef, PyResult, TypeProtocol};
use crate::VirtualMachine;

fn operator_length_hint(obj: PyObjectRef, default: OptionalArg, vm: &VirtualMachine) -> PyResult {
    let default = default.unwrap_or_else(|| vm.new_int(0));
    if !objtype::isinstance(&default, &vm.ctx.types.int_type) {
        return Err(vm.new_type_error(format!(
            "'{}' type cannot be interpreted as an integer",
            default.class().name
        )));
    }
    let hint = objiter::length_hint(vm, obj)?
        .map(|i| vm.new_int(i))
        .unwrap_or(default);
    Ok(hint)
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    py_module!(vm, "_operator", {
        "length_hint" => vm.ctx.new_rustfunc(operator_length_hint),
    })
}
