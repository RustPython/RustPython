use super::super::pyobject::{
    AttributeProtocol, PyContext, PyFuncArgs, PyObjectKind, PyObjectRef, PyResult, TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objint;
use super::objtype;

fn float_repr(vm: &mut VirtualMachine, args: PyFuncArgs) -> Result<PyObjectRef, PyObjectRef> {
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
    let val = if objtype::isinstance(arg, vm.ctx.float_type()) {
        get_value(arg)
    } else if objtype::isinstance(arg, vm.ctx.int_type()) {
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
    let result = if objtype::isinstance(other, vm.ctx.float_type()) {
        let other = get_value(other);
        zelf == other
    } else if objtype::isinstance(other, vm.ctx.int_type()) {
        let other = objint::get_value(other) as f64;
        zelf == other
    } else {
        false
    };
    Ok(vm.ctx.new_bool(result))
}

fn float_le(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [
            (zelf, Some(vm.ctx.float_type())),
            (other, Some(vm.ctx.float_type()))
        ]
    );
    let zelf = get_value(zelf);
    let other = get_value(other);
    let result = zelf <= other;
    Ok(vm.ctx.new_bool(result))
}

fn float_abs(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(i, Some(vm.ctx.float_type()))]);
    Ok(vm.ctx.new_float(get_value(i).abs()))
}

fn float_add(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(i, Some(vm.ctx.float_type())), (i2, None)]
    );

    let v1 = get_value(i);
    if objtype::isinstance(i2, vm.ctx.float_type()) {
        Ok(vm.ctx.new_float(v1 + get_value(i2)))
    } else if objtype::isinstance(i2, vm.ctx.int_type()) {
        Ok(vm.ctx.new_float(v1 + objint::get_value(i2) as f64))
    } else {
        Err(vm.new_type_error(format!("Cannot add {:?} and {:?}", i, i2)))
    }
}

fn float_divmod(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(i, Some(vm.ctx.float_type())), (i2, None)]
    );
    let args = PyFuncArgs::new(vec![i.clone(), i2.clone()], vec![]);
    if objtype::isinstance(i2, vm.ctx.float_type()) || objtype::isinstance(i2, vm.ctx.int_type()) {
        let r1 = float_floordiv(vm, args.clone());
        let r2 = float_mod(vm, args.clone());
        Ok(vm.ctx.new_tuple(vec![r1.unwrap(), r2.unwrap()]))
    } else {
        Err(vm.new_type_error(format!("Cannot divmod power {:?} and {:?}", i, i2)))
    }
}

fn float_floordiv(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(i, Some(vm.ctx.float_type())), (i2, None)]
    );
    if objtype::isinstance(i2, vm.ctx.float_type()) {
        Ok(vm.ctx.new_float((get_value(i) / get_value(i2)).floor()))
    } else if objtype::isinstance(i2, vm.ctx.int_type()) {
        Ok(vm
            .ctx
            .new_float((get_value(i) / objint::get_value(i2) as f64).floor()))
    } else {
        Err(vm.new_type_error(format!("Cannot floordiv {:?} and {:?}", i, i2)))
    }
}

fn float_sub(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(i, Some(vm.ctx.float_type())), (i2, None)]
    );
    let v1 = get_value(i);
    if objtype::isinstance(i2, vm.ctx.float_type()) {
        Ok(vm.ctx.new_float(v1 - get_value(i2)))
    } else if objtype::isinstance(i2, vm.ctx.int_type()) {
        Ok(vm.ctx.new_float(v1 - objint::get_value(i2) as f64))
    } else {
        Err(vm.new_type_error(format!("Cannot add {:?} and {:?}", i, i2)))
    }
}

fn float_mod(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(i, Some(vm.ctx.float_type())), (i2, None)]
    );
    if objtype::isinstance(i2, vm.ctx.float_type()) {
        Ok(vm.ctx.new_float(get_value(i) % get_value(i2)))
    } else if objtype::isinstance(i2, vm.ctx.int_type()) {
        Ok(vm
            .ctx
            .new_float(get_value(i) % objint::get_value(i2) as f64))
    } else {
        Err(vm.new_type_error(format!("Cannot mod {:?} and {:?}", i, i2)))
    }
}

fn float_pow(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(i, Some(vm.ctx.float_type())), (i2, None)]
    );

    let v1 = get_value(i);
    if objtype::isinstance(i2, vm.ctx.float_type()) {
        let result = v1.powf(get_value(i2));
        Ok(vm.ctx.new_float(result))
    } else if objtype::isinstance(i2, vm.ctx.int_type()) {
        let result = v1.powf(objint::get_value(i2) as f64);
        Ok(vm.ctx.new_float(result))
    } else {
        Err(vm.new_type_error(format!("Cannot add {:?} and {:?}", i, i2)))
    }
}

pub fn init(context: &PyContext) {
    let ref float_type = context.float_type;
    float_type.set_attr("__eq__", context.new_rustfunc(float_eq));
    float_type.set_attr("__le__", context.new_rustfunc(float_le));
    float_type.set_attr("__abs__", context.new_rustfunc(float_abs));
    float_type.set_attr("__add__", context.new_rustfunc(float_add));
    float_type.set_attr("__divmod__", context.new_rustfunc(float_divmod));
    float_type.set_attr("__floordiv__", context.new_rustfunc(float_floordiv));
    float_type.set_attr("__init__", context.new_rustfunc(float_init));
    float_type.set_attr("__mod__", context.new_rustfunc(float_mod));
    float_type.set_attr("__pow__", context.new_rustfunc(float_pow));
    float_type.set_attr("__sub__", context.new_rustfunc(float_sub));
    float_type.set_attr("__repr__", context.new_rustfunc(float_repr));
}
