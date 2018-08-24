use super::pyobject::{AttributeProtocol, PyContext, PyFuncArgs, PyObjectRef};
use super::vm::VirtualMachine;

fn str(vm: &mut VirtualMachine, _args: PyFuncArgs) -> Result<PyObjectRef, PyObjectRef> {
    // TODO: Implement objint::str
    Ok(vm.new_str("todo".to_string()))
}

/*
fn set_attr(a: &mut PyObjectRef, name: String, b: PyObjectRef) {
    a.borrow().dict.insert(name, b);
}
*/
pub fn init(context: &PyContext) {
    let ref int_type = context.int_type;
    int_type.set_attr("__str__", context.new_rustfunc(str));
}
