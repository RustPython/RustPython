use super::super::pyobject::{
    AttributeProtocol, PyContext, PyFuncArgs, PyObjectKind, PyObjectRef, PyResult, TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objint;
use super::objlist;
use super::objtype;
// Binary data support

// Fill bytes class methods:
pub fn init(context: &PyContext) {
    let ref bytes_type = context.bytes_type;
    bytes_type.set_attr("__eq__", context.new_rustfunc(bytes_eq));
    bytes_type.set_attr("__init__", context.new_rustfunc(bytes_init));
    bytes_type.set_attr("__repr__", context.new_rustfunc(bytes_repr));
}

// __init__ (store value into objectkind)
fn bytes_init(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.bytes_type())), (arg, None)]
    );
    let val = if objtype::isinstance(arg, vm.ctx.list_type()) {
        let mut data_bytes = vec![];
        for elem in objlist::get_elements(arg) {
            let v = match objint::to_int(vm, &elem, 10) {
                Ok(int_ref) => int_ref,
                Err(err) => return Err(err),
            };
            data_bytes.push(v as u8);
        }
        data_bytes
    } else {
        return Err(vm.new_type_error("Cannot construct bytes".to_string()));
    };
    set_value(zelf, val);
    Ok(vm.get_none())
}

fn bytes_eq(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(a, Some(vm.ctx.bytes_type())), (b, None)]
    );

    let result = if objtype::isinstance(b, vm.ctx.bytes_type()) {
        get_value(a) == get_value(b)
    } else {
        false
    };
    Ok(vm.ctx.new_bool(result))
}

pub fn get_value(obj: &PyObjectRef) -> Vec<u8> {
    if let PyObjectKind::Bytes { value } = &obj.borrow().kind {
        value.clone()
    } else {
        panic!("Inner error getting int {:?}", obj);
    }
}

fn set_value(obj: &PyObjectRef, value: Vec<u8>) {
    obj.borrow_mut().kind = PyObjectKind::Bytes { value };
}

fn bytes_repr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(obj, Some(vm.ctx.bytes_type()))]);
    let data = get_value(obj);
    let data: Vec<String> = data.into_iter().map(|b| format!("\\x{:02x}", b)).collect();
    let data = data.join("");
    Ok(vm.new_str(format!("b'{}'", data)))
}
