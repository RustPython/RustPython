use super::super::pyobject::{
    PyContext, PyFuncArgs, PyObjectPayload, PyObjectRef, PyResult, TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objint;
use super::objtype;
use num_traits::ToPrimitive;

fn float_repr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
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
    let val = if objtype::isinstance(arg, &vm.ctx.float_type()) {
        get_value(arg)
    } else if objtype::isinstance(arg, &vm.ctx.int_type()) {
        objint::get_value(arg).to_f64().unwrap()
    } else {
        return Err(vm.new_type_error("Cannot construct int".to_string()));
    };
    set_value(zelf, val);
    Ok(vm.get_none())
}

// Retrieve inner float value:
pub fn get_value(obj: &PyObjectRef) -> f64 {
    if let PyObjectPayload::Float { value } = &obj.borrow().payload {
        *value
    } else {
        panic!("Inner error getting float");
    }
}

pub fn make_float(vm: &mut VirtualMachine, obj: &PyObjectRef) -> Result<f64, PyObjectRef> {
    if objtype::isinstance(obj, &vm.ctx.float_type()) {
        Ok(get_value(obj))
    } else if let Ok(method) = vm.get_method(obj.clone(), "__float__") {
        let res = vm.invoke(
            method,
            PyFuncArgs {
                args: vec![],
                kwargs: vec![],
            },
        )?;
        Ok(get_value(&res))
    } else {
        Err(vm.new_type_error(format!("Cannot cast {} to float", obj.borrow())))
    }
}

fn set_value(obj: &PyObjectRef, value: f64) {
    obj.borrow_mut().payload = PyObjectPayload::Float { value };
}

fn float_eq(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.float_type())), (other, None)]
    );
    let zelf = get_value(zelf);
    let result = if objtype::isinstance(other, &vm.ctx.float_type()) {
        let other = get_value(other);
        zelf == other
    } else if objtype::isinstance(other, &vm.ctx.int_type()) {
        let other = objint::get_value(other).to_f64().unwrap();
        zelf == other
    } else {
        false
    };
    Ok(vm.ctx.new_bool(result))
}

fn float_lt(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(i, Some(vm.ctx.float_type())), (i2, None)]
    );

    let v1 = get_value(i);
    if objtype::isinstance(i2, &vm.ctx.float_type()) {
        Ok(vm.ctx.new_bool(v1 < get_value(i2)))
    } else if objtype::isinstance(i2, &vm.ctx.int_type()) {
        Ok(vm
            .ctx
            .new_bool(v1 < objint::get_value(i2).to_f64().unwrap()))
    } else {
        Err(vm.new_type_error(format!("Cannot compare {} and {}", i.borrow(), i2.borrow())))
    }
}

fn float_le(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(i, Some(vm.ctx.float_type())), (i2, None)]
    );

    let v1 = get_value(i);
    if objtype::isinstance(i2, &vm.ctx.float_type()) {
        Ok(vm.ctx.new_bool(v1 <= get_value(i2)))
    } else if objtype::isinstance(i2, &vm.ctx.int_type()) {
        Ok(vm
            .ctx
            .new_bool(v1 <= objint::get_value(i2).to_f64().unwrap()))
    } else {
        Err(vm.new_type_error(format!("Cannot compare {} and {}", i.borrow(), i2.borrow())))
    }
}

fn float_gt(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(i, Some(vm.ctx.float_type())), (i2, None)]
    );

    let v1 = get_value(i);
    if objtype::isinstance(i2, &vm.ctx.float_type()) {
        Ok(vm.ctx.new_bool(v1 > get_value(i2)))
    } else if objtype::isinstance(i2, &vm.ctx.int_type()) {
        Ok(vm
            .ctx
            .new_bool(v1 > objint::get_value(i2).to_f64().unwrap()))
    } else {
        Err(vm.new_type_error(format!("Cannot compare {} and {}", i.borrow(), i2.borrow())))
    }
}

fn float_ge(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(i, Some(vm.ctx.float_type())), (i2, None)]
    );

    let v1 = get_value(i);
    if objtype::isinstance(i2, &vm.ctx.float_type()) {
        Ok(vm.ctx.new_bool(v1 >= get_value(i2)))
    } else if objtype::isinstance(i2, &vm.ctx.int_type()) {
        Ok(vm
            .ctx
            .new_bool(v1 >= objint::get_value(i2).to_f64().unwrap()))
    } else {
        Err(vm.new_type_error(format!("Cannot compare {} and {}", i.borrow(), i2.borrow())))
    }
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
    if objtype::isinstance(i2, &vm.ctx.float_type()) {
        Ok(vm.ctx.new_float(v1 + get_value(i2)))
    } else if objtype::isinstance(i2, &vm.ctx.int_type()) {
        Ok(vm
            .ctx
            .new_float(v1 + objint::get_value(i2).to_f64().unwrap()))
    } else {
        Err(vm.new_type_error(format!("Cannot add {} and {}", i.borrow(), i2.borrow())))
    }
}

fn float_divmod(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(i, Some(vm.ctx.float_type())), (i2, None)]
    );
    let args = PyFuncArgs::new(vec![i.clone(), i2.clone()], vec![]);
    if objtype::isinstance(i2, &vm.ctx.float_type()) || objtype::isinstance(i2, &vm.ctx.int_type())
    {
        let r1 = float_floordiv(vm, args.clone())?;
        let r2 = float_mod(vm, args.clone())?;
        Ok(vm.ctx.new_tuple(vec![r1, r2]))
    } else {
        Err(vm.new_type_error(format!(
            "Cannot divmod power {} and {}",
            i.borrow(),
            i2.borrow()
        )))
    }
}

fn float_floordiv(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(i, Some(vm.ctx.float_type())), (i2, None)]
    );

    let v1 = get_value(i);
    let v2 = if objtype::isinstance(i2, &vm.ctx.float_type) {
        get_value(i2)
    } else if objtype::isinstance(i2, &vm.ctx.int_type) {
        objint::get_value(i2)
            .to_f64()
            .ok_or_else(|| vm.new_overflow_error("int too large to convert to float".to_string()))?
    } else {
        return Err(vm.new_type_error(format!(
            "Cannot floordiv {} and {}",
            i.borrow(),
            i2.borrow()
        )));
    };

    if v2 != 0.0 {
        Ok(vm.ctx.new_float((v1 / v2).floor()))
    } else {
        Err(vm.new_zero_division_error("float floordiv by zero".to_string()))
    }
}

fn float_sub(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(i, Some(vm.ctx.float_type())), (i2, None)]
    );
    let v1 = get_value(i);
    if objtype::isinstance(i2, &vm.ctx.float_type()) {
        Ok(vm.ctx.new_float(v1 - get_value(i2)))
    } else if objtype::isinstance(i2, &vm.ctx.int_type()) {
        Ok(vm
            .ctx
            .new_float(v1 - objint::get_value(i2).to_f64().unwrap()))
    } else {
        Err(vm.new_type_error(format!("Cannot add {} and {}", i.borrow(), i2.borrow())))
    }
}

fn float_mod(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(i, Some(vm.ctx.float_type())), (i2, None)]
    );

    let v1 = get_value(i);
    let v2 = if objtype::isinstance(i2, &vm.ctx.float_type) {
        get_value(i2)
    } else if objtype::isinstance(i2, &vm.ctx.int_type) {
        objint::get_value(i2)
            .to_f64()
            .ok_or_else(|| vm.new_overflow_error("int too large to convert to float".to_string()))?
    } else {
        return Err(vm.new_type_error(format!("Cannot mod {} and {}", i.borrow(), i2.borrow())));
    };

    if v2 != 0.0 {
        Ok(vm.ctx.new_float(v1 % v2))
    } else {
        Err(vm.new_zero_division_error("float mod by zero".to_string()))
    }
}

fn float_neg(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(i, Some(vm.ctx.float_type()))]);

    let v1 = get_value(i);
    Ok(vm.ctx.new_float(-v1))
}

fn float_pow(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(i, Some(vm.ctx.float_type())), (i2, None)]
    );

    let v1 = get_value(i);
    if objtype::isinstance(i2, &vm.ctx.float_type()) {
        let result = v1.powf(get_value(i2));
        Ok(vm.ctx.new_float(result))
    } else if objtype::isinstance(i2, &vm.ctx.int_type()) {
        let result = v1.powf(objint::get_value(i2).to_f64().unwrap());
        Ok(vm.ctx.new_float(result))
    } else {
        Err(vm.new_type_error(format!("Cannot add {} and {}", i.borrow(), i2.borrow())))
    }
}

fn float_truediv(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(i, Some(vm.ctx.float_type())), (i2, None)]
    );

    let v1 = get_value(i);
    let v2 = if objtype::isinstance(i2, &vm.ctx.float_type) {
        get_value(i2)
    } else if objtype::isinstance(i2, &vm.ctx.int_type) {
        objint::get_value(i2)
            .to_f64()
            .ok_or_else(|| vm.new_overflow_error("int too large to convert to float".to_string()))?
    } else {
        return Err(vm.new_type_error(format!("Cannot divide {} and {}", i.borrow(), i2.borrow())));
    };

    if v2 != 0.0 {
        Ok(vm.ctx.new_float(v1 / v2))
    } else {
        Err(vm.new_zero_division_error("float division by zero".to_string()))
    }
}

fn float_mul(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(i, Some(vm.ctx.float_type())), (i2, None)]
    );
    let v1 = get_value(i);
    if objtype::isinstance(i2, &vm.ctx.float_type) {
        Ok(vm.ctx.new_float(v1 * get_value(i2)))
    } else if objtype::isinstance(i2, &vm.ctx.int_type) {
        Ok(vm
            .ctx
            .new_float(v1 * objint::get_value(i2).to_f64().unwrap()))
    } else {
        Err(vm.new_type_error(format!(
            "Cannot multiply {} and {}",
            i.borrow(),
            i2.borrow()
        )))
    }
}

pub fn init(context: &PyContext) {
    let float_type = &context.float_type;

    let float_doc = "Convert a string or number to a floating point number, if possible.";

    context.set_attr(&float_type, "__eq__", context.new_rustfunc(float_eq));
    context.set_attr(&float_type, "__lt__", context.new_rustfunc(float_lt));
    context.set_attr(&float_type, "__le__", context.new_rustfunc(float_le));
    context.set_attr(&float_type, "__gt__", context.new_rustfunc(float_gt));
    context.set_attr(&float_type, "__ge__", context.new_rustfunc(float_ge));
    context.set_attr(&float_type, "__abs__", context.new_rustfunc(float_abs));
    context.set_attr(&float_type, "__add__", context.new_rustfunc(float_add));
    context.set_attr(
        &float_type,
        "__divmod__",
        context.new_rustfunc(float_divmod),
    );
    context.set_attr(
        &float_type,
        "__floordiv__",
        context.new_rustfunc(float_floordiv),
    );
    context.set_attr(&float_type, "__init__", context.new_rustfunc(float_init));
    context.set_attr(&float_type, "__mod__", context.new_rustfunc(float_mod));
    context.set_attr(&float_type, "__neg__", context.new_rustfunc(float_neg));
    context.set_attr(&float_type, "__pow__", context.new_rustfunc(float_pow));
    context.set_attr(&float_type, "__sub__", context.new_rustfunc(float_sub));
    context.set_attr(&float_type, "__repr__", context.new_rustfunc(float_repr));
    context.set_attr(
        &float_type,
        "__doc__",
        context.new_str(float_doc.to_string()),
    );
    context.set_attr(
        &float_type,
        "__truediv__",
        context.new_rustfunc(float_truediv),
    );
    context.set_attr(&float_type, "__mul__", context.new_rustfunc(float_mul));
}
