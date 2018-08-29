use super::objint;
use super::objtype;
use super::pyobject::{
    AttributeProtocol, PyContext, PyFuncArgs, PyObjectKind, PyObjectRef, PyResult, TypeProtocol,
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

fn float_add(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(i, Some(vm.ctx.float_type())), (i2, None)]
    );

    if objtype::isinstance(i2.clone(), vm.ctx.float_type()) {
        Ok(vm
            .ctx
            .new_float(get_value(i.clone()) + get_value(i2.clone())))
    } else if objtype::isinstance(i2.clone(), vm.ctx.int_type()) {
        Ok(vm
            .ctx
            .new_float(get_value(i.clone()) + objint::get_value(i2.clone()) as f64))
    } else {
        Err(vm.new_type_error(format!("Cannot add {:?} and {:?}", i, i2)))
    }
}

fn float_sub(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(i, Some(vm.ctx.float_type())), (i2, None)]
    );

    if objtype::isinstance(i2.clone(), vm.ctx.float_type()) {
        Ok(vm
            .ctx
            .new_float(get_value(i.clone()) - get_value(i2.clone())))
    } else if objtype::isinstance(i2.clone(), vm.ctx.int_type()) {
        Ok(vm
            .ctx
            .new_float(get_value(i.clone()) - objint::get_value(i2.clone()) as f64))
    } else {
        Err(vm.new_type_error(format!("Cannot add {:?} and {:?}", i, i2)))
    }
}

pub fn init(context: &PyContext) {
    let ref float_type = context.float_type;
    float_type.set_attr("__add__", context.new_rustfunc(float_add));
    float_type.set_attr("__str__", context.new_rustfunc(str));
    float_type.set_attr("__sub__", context.new_rustfunc(float_sub));
    float_type.set_attr("__repr__", context.new_rustfunc(str));
}
