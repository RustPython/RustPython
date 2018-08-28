use super::objtype;
use super::pyobject::{
    AttributeProtocol, PyContext, PyFuncArgs, PyObjectKind, PyObjectRef, TypeProtocol,
};
use super::vm::VirtualMachine;

fn str(vm: &mut VirtualMachine, args: PyFuncArgs) -> Result<PyObjectRef, PyObjectRef> {
    arg_check!(vm, args, required = [(float, Some(vm.ctx.float_type()))]);
    let v = get_value(float.clone());
    Ok(vm.new_str(v.to_string()))
}

// Retrieve inner float value:
pub fn get_value(obj: PyObjectRef) -> f64 {
    if let PyObjectKind::Float { value } = &obj.borrow().kind {
        *value
    } else {
        panic!("Inner error getting float");
    }
}

pub fn init(context: &PyContext) {
    let ref float_type = context.float_type;
    float_type.set_attr("__str__", context.new_rustfunc(str));
    float_type.set_attr("__repr__", context.new_rustfunc(str));
}
