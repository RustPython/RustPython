use super::objtype;
use super::pyobject::{
    AttributeProtocol, PyContext, PyFuncArgs, PyObjectKind, PyObjectRef, TypeProtocol,
};
use super::vm::VirtualMachine;

fn str(vm: &mut VirtualMachine, args: PyFuncArgs) -> Result<PyObjectRef, PyObjectRef> {
    arg_check!(vm, args, required = [(int, Some(vm.ctx.int_type()))]);
    let v = get_value(int.clone());
    Ok(vm.new_str(v.to_string()))
}

// Retrieve inner int value:
fn get_value(obj: PyObjectRef) -> i32 {
    if let PyObjectKind::Integer { value } = &obj.borrow().kind {
        *value
    } else {
        panic!("Inner error getting int");
    }
}

pub fn init(context: &PyContext) {
    let ref int_type = context.int_type;
    int_type.set_attr("__str__", context.new_rustfunc(str));
    int_type.set_attr("__repr__", context.new_rustfunc(str));
}
