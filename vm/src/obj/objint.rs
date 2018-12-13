use super::super::pyobject::{
    FromPyObjectRef, PyContext, PyFuncArgs, PyObject, PyObjectKind, PyObjectRef, PyResult,
    TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objfloat;
use super::objstr;
use super::objtype;
use num_bigint::{BigInt, ToBigInt};
use num_traits::{Pow, Signed, ToPrimitive, Zero};
use std::hash::{Hash, Hasher};

// This proxy allows for easy switching between types.
type IntType = BigInt;

fn int_repr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(int, Some(vm.ctx.int_type()))]);
    let v = get_value(int);
    Ok(vm.new_str(v.to_string()))
}

fn int_new(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(cls, None)],
        optional = [(val_option, None)]
    );
    if !objtype::issubclass(cls, &vm.ctx.int_type()) {
        return Err(vm.new_type_error(format!("{:?} is not a subtype of int", cls)));
    }

    // TODO: extract kwargs:
    let base = 10;
    let val = match val_option {
        Some(val) => to_int(vm, val, base)?,
        None => Zero::zero(),
    };
    Ok(PyObject::new(
        PyObjectKind::Integer { value: val },
        cls.clone(),
    ))
}

// Casting function:
pub fn to_int(
    vm: &mut VirtualMachine,
    obj: &PyObjectRef,
    base: u32,
) -> Result<IntType, PyObjectRef> {
    let val = if objtype::isinstance(obj, &vm.ctx.int_type()) {
        get_value(obj)
    } else if objtype::isinstance(obj, &vm.ctx.float_type()) {
        objfloat::get_value(obj).to_bigint().unwrap()
    } else if objtype::isinstance(obj, &vm.ctx.str_type()) {
        let s = objstr::get_value(obj);
        match i32::from_str_radix(&s, base) {
            Ok(v) => v.to_bigint().unwrap(),
            Err(err) => {
                trace!("Error occured during int conversion {:?}", err);
                return Err(vm.new_value_error(format!(
                    "invalid literal for int() with base {}: '{}'",
                    base, s
                )));
            }
        }
    } else {
        let type_name = objtype::get_type_name(&obj.typ());
        return Err(vm.new_type_error(format!(
            "int() argument must be a string or a number, not '{}'",
            type_name
        )));
    };
    Ok(val)
}

// Retrieve inner int value:
pub fn get_value(obj: &PyObjectRef) -> IntType {
    if let PyObjectKind::Integer { value } = &obj.borrow().kind {
        value.clone()
    } else {
        panic!("Inner error getting int {:?}", obj);
    }
}

impl FromPyObjectRef for BigInt {
    fn from_pyobj(obj: &PyObjectRef) -> BigInt {
        get_value(obj)
    }
}

fn int_eq(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.int_type())), (other, None)]
    );
    let result = if objtype::isinstance(other, &vm.ctx.int_type()) {
        let zelf = BigInt::from_pyobj(zelf);
        let other = BigInt::from_pyobj(other);
        zelf == other
    } else if objtype::isinstance(other, &vm.ctx.float_type()) {
        let zelf = BigInt::from_pyobj(zelf).to_f64().unwrap();
        let other = objfloat::get_value(other);
        zelf == other
    } else {
        false
    };
    Ok(vm.ctx.new_bool(result))
}

fn int_lt(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [
            (zelf, Some(vm.ctx.int_type())),
            (other, Some(vm.ctx.int_type()))
        ]
    );
    let zelf = BigInt::from_pyobj(zelf);
    let other = BigInt::from_pyobj(other);
    let result = zelf < other;
    Ok(vm.ctx.new_bool(result))
}

fn int_le(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [
            (zelf, Some(vm.ctx.int_type())),
            (other, Some(vm.ctx.int_type()))
        ]
    );
    let zelf = BigInt::from_pyobj(zelf);
    let other = BigInt::from_pyobj(other);
    let result = zelf <= other;
    Ok(vm.ctx.new_bool(result))
}

fn int_gt(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [
            (zelf, Some(vm.ctx.int_type())),
            (other, Some(vm.ctx.int_type()))
        ]
    );
    let zelf = BigInt::from_pyobj(zelf);
    let other = BigInt::from_pyobj(other);
    let result = zelf > other;
    Ok(vm.ctx.new_bool(result))
}

fn int_ge(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [
            (zelf, Some(vm.ctx.int_type())),
            (other, Some(vm.ctx.int_type()))
        ]
    );
    let zelf = BigInt::from_pyobj(zelf);
    let other = BigInt::from_pyobj(other);
    let result = zelf >= other;
    Ok(vm.ctx.new_bool(result))
}

fn int_hash(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, Some(vm.ctx.int_type()))]);
    let value = BigInt::from_pyobj(zelf);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    let hash = hasher.finish();
    Ok(vm.ctx.new_int(hash.to_bigint().unwrap()))
}

fn int_abs(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(i, Some(vm.ctx.int_type()))]);
    Ok(vm.ctx.new_int(get_value(i).abs()))
}

fn int_add(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(i, Some(vm.ctx.int_type())), (i2, None)]
    );
    let i = BigInt::from_pyobj(i);
    if objtype::isinstance(i2, &vm.ctx.int_type()) {
        Ok(vm.ctx.new_int(i + get_value(i2)))
    } else if objtype::isinstance(i2, &vm.ctx.float_type()) {
        Ok(vm
            .ctx
            .new_float(i.to_f64().unwrap() + objfloat::get_value(i2)))
    } else {
        Err(vm.new_type_error(format!("Cannot add {:?} and {:?}", i, i2)))
    }
}

fn int_float(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(i, Some(vm.ctx.int_type()))]);
    let i = get_value(i);
    Ok(vm.ctx.new_float(i.to_f64().unwrap()))
}

fn int_floordiv(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(i, Some(vm.ctx.int_type())), (i2, None)]
    );
    if objtype::isinstance(i2, &vm.ctx.int_type()) {
        Ok(vm.ctx.new_int(get_value(i) / get_value(i2)))
    } else {
        Err(vm.new_type_error(format!("Cannot floordiv {:?} and {:?}", i, i2)))
    }
}

fn int_sub(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(i, Some(vm.ctx.int_type())), (i2, None)]
    );
    let i = BigInt::from_pyobj(i);
    if objtype::isinstance(i2, &vm.ctx.int_type()) {
        Ok(vm.ctx.new_int(i - get_value(i2)))
    } else if objtype::isinstance(i2, &vm.ctx.float_type()) {
        Ok(vm
            .ctx
            .new_float(i.to_f64().unwrap() - objfloat::get_value(i2)))
    } else {
        Err(vm.new_not_implemented_error(format!("Cannot substract {:?} and {:?}", i, i2)))
    }
}

fn int_mul(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(i, Some(vm.ctx.int_type())), (i2, None)]
    );
    if objtype::isinstance(i2, &vm.ctx.int_type()) {
        Ok(vm.ctx.new_int(get_value(i) * get_value(i2)))
    } else if objtype::isinstance(i2, &vm.ctx.float_type()) {
        Ok(vm
            .ctx
            .new_float(get_value(i).to_f64().unwrap() * objfloat::get_value(i2)))
    } else {
        Err(vm.new_type_error(format!("Cannot multiply {:?} and {:?}", i, i2)))
    }
}

fn int_truediv(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(i, Some(vm.ctx.int_type())), (i2, None)]
    );
    let v1 = get_value(i);
    if objtype::isinstance(i2, &vm.ctx.int_type()) {
        Ok(vm
            .ctx
            .new_float(v1.to_f64().unwrap() / get_value(i2).to_f64().unwrap()))
    } else if objtype::isinstance(i2, &vm.ctx.float_type()) {
        Ok(vm
            .ctx
            .new_float(v1.to_f64().unwrap() / objfloat::get_value(i2)))
    } else {
        Err(vm.new_type_error(format!("Cannot divide {:?} and {:?}", i, i2)))
    }
}

fn int_mod(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(i, Some(vm.ctx.int_type())), (i2, None)]
    );
    let v1 = get_value(i);
    if objtype::isinstance(i2, &vm.ctx.int_type()) {
        Ok(vm.ctx.new_int(v1 % get_value(i2)))
    } else {
        Err(vm.new_type_error(format!("Cannot modulo {:?} and {:?}", i, i2)))
    }
}

fn int_neg(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(i, Some(vm.ctx.int_type()))]);
    let i = BigInt::from_pyobj(i);
    Ok(vm.ctx.new_int(-i))
}

fn int_pos(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(i, Some(vm.ctx.int_type()))]);
    Ok(i.clone())
}

fn int_pow(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(i, Some(vm.ctx.int_type())), (i2, None)]
    );
    let v1 = get_value(i);
    if objtype::isinstance(i2, &vm.ctx.int_type()) {
        let v2 = get_value(i2).to_u32().unwrap();
        Ok(vm.ctx.new_int(v1.pow(v2).to_bigint().unwrap()))
    } else if objtype::isinstance(i2, &vm.ctx.float_type()) {
        let v2 = objfloat::get_value(i2);
        Ok(vm.ctx.new_float((v1.to_f64().unwrap()).powf(v2)))
    } else {
        Err(vm.new_type_error(format!("Cannot raise power {:?} and {:?}", i, i2)))
    }
}

fn int_divmod(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(i, Some(vm.ctx.int_type())), (i2, None)]
    );
    let args = PyFuncArgs::new(vec![i.clone(), i2.clone()], vec![]);
    if objtype::isinstance(i2, &vm.ctx.int_type()) {
        let r1 = int_floordiv(vm, args.clone());
        let r2 = int_mod(vm, args.clone());
        Ok(vm.ctx.new_tuple(vec![r1.unwrap(), r2.unwrap()]))
    } else {
        Err(vm.new_type_error(format!("Cannot divmod power {:?} and {:?}", i, i2)))
    }
}

fn int_xor(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(i, Some(vm.ctx.int_type())), (i2, None)]
    );
    let v1 = get_value(i);
    if objtype::isinstance(i2, &vm.ctx.int_type()) {
        let v2 = get_value(i2);
        Ok(vm.ctx.new_int(v1 ^ v2))
    } else {
        Err(vm.new_type_error(format!("Cannot xor {:?} and {:?}", i, i2)))
    }
}

fn int_or(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(i, Some(vm.ctx.int_type())), (i2, None)]
    );
    let v1 = get_value(i);
    if objtype::isinstance(i2, &vm.ctx.int_type()) {
        let v2 = get_value(i2);
        Ok(vm.ctx.new_int(v1 | v2))
    } else {
        Err(vm.new_type_error(format!("Cannot or {:?} and {:?}", i, i2)))
    }
}

fn int_and(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(i, Some(vm.ctx.int_type())), (i2, None)]
    );
    let v1 = get_value(i);
    if objtype::isinstance(i2, &vm.ctx.int_type()) {
        let v2 = get_value(i2);
        Ok(vm.ctx.new_int(v1 & v2))
    } else {
        Err(vm.new_type_error(format!("Cannot and {:?} and {:?}", i, i2)))
    }
}

fn int_bit_length(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(i, Some(vm.ctx.int_type()))]);
    let v = get_value(i);
    let bits = v.bits();
    Ok(vm.ctx.new_int(bits.to_bigint().unwrap()))
}

pub fn init(context: &PyContext) {
    let ref int_type = context.int_type;
    context.set_attr(&int_type, "__eq__", context.new_rustfunc(int_eq));
    context.set_attr(&int_type, "__lt__", context.new_rustfunc(int_lt));
    context.set_attr(&int_type, "__le__", context.new_rustfunc(int_le));
    context.set_attr(&int_type, "__gt__", context.new_rustfunc(int_gt));
    context.set_attr(&int_type, "__ge__", context.new_rustfunc(int_ge));
    context.set_attr(&int_type, "__abs__", context.new_rustfunc(int_abs));
    context.set_attr(&int_type, "__add__", context.new_rustfunc(int_add));
    context.set_attr(&int_type, "__and__", context.new_rustfunc(int_and));
    context.set_attr(&int_type, "__divmod__", context.new_rustfunc(int_divmod));
    context.set_attr(&int_type, "__float__", context.new_rustfunc(int_float));
    context.set_attr(
        &int_type,
        "__floordiv__",
        context.new_rustfunc(int_floordiv),
    );
    context.set_attr(&int_type, "__hash__", context.new_rustfunc(int_hash));
    context.set_attr(&int_type, "__new__", context.new_rustfunc(int_new));
    context.set_attr(&int_type, "__mod__", context.new_rustfunc(int_mod));
    context.set_attr(&int_type, "__mul__", context.new_rustfunc(int_mul));
    context.set_attr(&int_type, "__neg__", context.new_rustfunc(int_neg));
    context.set_attr(&int_type, "__or__", context.new_rustfunc(int_or));
    context.set_attr(&int_type, "__pos__", context.new_rustfunc(int_pos));
    context.set_attr(&int_type, "__pow__", context.new_rustfunc(int_pow));
    context.set_attr(&int_type, "__repr__", context.new_rustfunc(int_repr));
    context.set_attr(&int_type, "__sub__", context.new_rustfunc(int_sub));
    context.set_attr(&int_type, "__truediv__", context.new_rustfunc(int_truediv));
    context.set_attr(&int_type, "__xor__", context.new_rustfunc(int_xor));
    context.set_attr(
        &int_type,
        "bit_length",
        context.new_rustfunc(int_bit_length),
    );
}
