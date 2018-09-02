use super::super::pyobject::{
    AttributeProtocol, PyContext, PyFuncArgs, PyObjectKind, PyObjectRef, PyResult, TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objint;
use super::objtype;

fn str(vm: &mut VirtualMachine, args: PyFuncArgs) -> Result<PyObjectRef, PyObjectRef> {
    arg_check!(vm, args, required = [(float, Some(vm.ctx.float_type()))]);
    let v = get_value(float);
    Ok(vm.new_str(v.to_string()))
}

// __init__()
fn float_init(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.float_type())), (arg, None)]
    );
    let val = if objtype::isinstance(arg.clone(), vm.ctx.float_type()) {
        get_value(arg)
    } else if objtype::isinstance(arg.clone(), vm.ctx.int_type()) {
        objint::get_value(arg) as f64
    } else {
        return Err(vm.new_type_error("Cannot construct int".to_string()));
    };
    set_value(zelf, val);
    Ok(vm.get_none())
}

// Retrieve inner float value:
pub fn get_value(obj: &PyObjectRef) -> f64 {
    if let PyObjectKind::Float { value } = &obj.borrow().kind {
        *value
    } else {
        panic!("Inner error getting float");
    }
}

fn set_value(obj: &PyObjectRef, value: f64) {
    obj.borrow_mut().kind = PyObjectKind::Float { value };
}

fn float_eq(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.float_type())), (other, None)]
    );
    let zelf = get_value(zelf);
    let result = if objtype::isinstance(other.clone(), vm.ctx.float_type()) {
        let other = get_value(other);
        zelf == other
    } else if objtype::isinstance(other.clone(), vm.ctx.int_type()) {
        let other = objint::get_value(other) as f64;
        zelf == other
    } else {
        false
    };
    Ok(vm.ctx.new_bool(result))
}

fn float_add(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(i, Some(vm.ctx.float_type())), (i2, None)]
    );

    let v1 = get_value(i);
    if objtype::isinstance(i2.clone(), vm.ctx.float_type()) {
        Ok(vm.ctx.new_float(v1 + get_value(i2)))
    } else if objtype::isinstance(i2.clone(), vm.ctx.int_type()) {
        Ok(vm.ctx.new_float(v1 + objint::get_value(i2) as f64))
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

    let v1 = get_value(i);
    if objtype::isinstance(i2.clone(), vm.ctx.float_type()) {
        Ok(vm.ctx.new_float(v1 - get_value(i2)))
    } else if objtype::isinstance(i2.clone(), vm.ctx.int_type()) {
        Ok(vm.ctx.new_float(v1 - objint::get_value(i2) as f64))
    } else {
        Err(vm.new_type_error(format!("Cannot add {:?} and {:?}", i, i2)))
    }
}

fn float_pow(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(i, Some(vm.ctx.float_type())), (i2, None)]
    );

    let v1 = get_value(i);
    if objtype::isinstance(i2.clone(), vm.ctx.float_type()) {
        let result = v1.powf(get_value(i2));
        Ok(vm.ctx.new_float(result))
    } else if objtype::isinstance(i2.clone(), vm.ctx.int_type()) {
        let result = v1.powf(objint::get_value(i2) as f64);
        Ok(vm.ctx.new_float(result))
    } else {
        Err(vm.new_type_error(format!("Cannot add {:?} and {:?}", i, i2)))
    }
}

pub fn init(context: &PyContext) {
    let ref float_type = context.float_type;
    float_type.set_attr("__eq__", context.new_rustfunc(float_eq));
    float_type.set_attr("__add__", context.new_rustfunc(float_add));
    float_type.set_attr("__init__", context.new_rustfunc(float_init));
    float_type.set_attr("__pow__", context.new_rustfunc(float_pow));
    float_type.set_attr("__str__", context.new_rustfunc(str));
    float_type.set_attr("__sub__", context.new_rustfunc(float_sub));
    float_type.set_attr("__repr__", context.new_rustfunc(str));
}
