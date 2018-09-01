use super::super::objsequence::{get_elements, seq_equal};
use super::super::pyobject::{
    AttributeProtocol, PyContext, PyFuncArgs, PyObjectRef, PyResult, TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objstr;
use super::objtype;

fn tuple_eq(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.tuple_type())), (other, None)]
    );

    let result = if objtype::isinstance(other.clone(), vm.ctx.tuple_type()) {
        let zelf = get_elements(zelf.clone());
        let other = get_elements(other.clone());
        seq_equal(vm, zelf, other)?
    } else {
        false
    };
    Ok(vm.ctx.new_bool(result))
}

fn tuple_len(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    trace!("tuple.len called with: {:?}", args);
    arg_check!(vm, args, required = [(zelf, Some(vm.ctx.tuple_type()))]);
    let elements = get_elements(zelf.clone());
    Ok(vm.context().new_int(elements.len() as i32))
}

fn tuple_str(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, Some(vm.ctx.tuple_type()))]);

    let elements = get_elements(zelf.clone());
    if elements.len() == 1 {
        let ref part = vm.to_str(elements[0].clone())?;
        let s = format!("({},)", objstr::get_value(part));
        return Ok(vm.new_str(s));
    }
    let mut str_parts = vec![];
    for elem in elements {
        match vm.to_str(elem) {
            Ok(s) => str_parts.push(objstr::get_value(&s)),
            Err(err) => return Err(err),
        }
    }

    let s = format!("({})", str_parts.join(", "));
    Ok(vm.new_str(s))
}

pub fn init(context: &PyContext) {
    let ref tuple_type = context.tuple_type;
    tuple_type.set_attr("__eq__", context.new_rustfunc(tuple_eq));
    tuple_type.set_attr("__len__", context.new_rustfunc(tuple_len));
    tuple_type.set_attr("__str__", context.new_rustfunc(tuple_str));
}
