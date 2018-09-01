use super::super::pyobject::{
    AttributeProtocol, PyContext, PyFuncArgs, PyObjectKind, PyObjectRef, PyResult, TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objstr;
use super::objtype;

fn tuple_str(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(o, Some(vm.ctx.tuple_type()))]);

    let elements = get_elements(o);
    let mut str_parts = vec![];
    for elem in elements {
        match vm.to_str(elem) {
            Ok(s) => str_parts.push(objstr::get_value(&s)),
            Err(err) => return Err(err),
        }
    }

    let s = if str_parts.len() == 1 {
        format!("({},)", str_parts.join(", "))
    } else {
        format!("({})", str_parts.join(", "))
    };
    Ok(vm.new_str(s))
}

pub fn get_elements(obj: &PyObjectRef) -> Vec<PyObjectRef> {
    if let PyObjectKind::List { elements } = &obj.borrow().kind {
        elements.to_vec()
    } else {
        panic!("Cannot extract elements from non-tuple");
    }
}

fn tuple_len(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(tuple, Some(vm.ctx.tuple_type()))]);
    let elements = get_elements(tuple);
    Ok(vm.context().new_int(elements.len() as i32))
}

pub fn init(context: &PyContext) {
    let ref tuple_type = context.tuple_type;
    tuple_type.set_attr("__len__", context.new_rustfunc(tuple_len));
    tuple_type.set_attr("__str__", context.new_rustfunc(tuple_str));
}
