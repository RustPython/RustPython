use super::objfloat;
use super::objstr;
use super::objtype;
use crate::format::FormatSpec;
use crate::pyobject::{
    FromPyObject, FromPyObjectRef, IntoPyObject, PyContext, PyFuncArgs, PyObject, PyObjectPayload,
    PyObjectRef, PyResult, TypeProtocol,
};
use crate::vm::VirtualMachine;
use num_bigint::{BigInt, ToBigInt};
use num_integer::Integer;
use num_traits::{Pow, Signed, ToPrimitive, Zero};
use std::hash::{Hash, Hasher};

// This proxy allows for easy switching between types.
type IntType = BigInt;

pub type PyInt = BigInt;

impl IntoPyObject for PyInt {
    fn into_pyobject(self, ctx: &PyContext) -> PyResult {
        Ok(ctx.new_int(self))
    }
}

// TODO: macro to impl for all primitive ints

impl IntoPyObject for usize {
    fn into_pyobject(self, ctx: &PyContext) -> PyResult {
        Ok(ctx.new_int(self))
    }
}

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
        PyObjectPayload::Integer { value: val },
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
                trace!("Error occurred during int conversion {:?}", err);
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
    if let PyObjectPayload::Integer { value } = &obj.borrow().payload {
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

fn int_bool(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, Some(vm.ctx.int_type()))]);
    let result = !BigInt::from_pyobj(zelf).is_zero();
    Ok(vm.ctx.new_bool(result))
}

fn int_invert(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, Some(vm.ctx.int_type()))]);

    let result = !BigInt::from_pyobj(zelf);

    Ok(vm.ctx.new_int(result))
}

fn int_eq(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.int_type())), (other, None)]
    );

    let zelf = BigInt::from_pyobj(zelf);
    let result = if objtype::isinstance(other, &vm.ctx.int_type()) {
        let other = BigInt::from_pyobj(other);
        zelf == other
    } else {
        return Ok(vm.ctx.not_implemented());
    };
    Ok(vm.ctx.new_bool(result))
}

fn int_ne(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.int_type())), (other, None)]
    );

    let zelf = BigInt::from_pyobj(zelf);
    let result = if objtype::isinstance(other, &vm.ctx.int_type()) {
        let other = BigInt::from_pyobj(other);
        zelf != other
    } else {
        return Ok(vm.ctx.not_implemented());
    };
    Ok(vm.ctx.new_bool(result))
}

fn int_lt(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.int_type())), (other, None)]
    );

    if !objtype::isinstance(other, &vm.ctx.int_type()) {
        return Ok(vm.ctx.not_implemented());
    }

    let zelf = BigInt::from_pyobj(zelf);
    let other = BigInt::from_pyobj(other);
    let result = zelf < other;
    Ok(vm.ctx.new_bool(result))
}

fn int_le(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.int_type())), (other, None)]
    );

    if !objtype::isinstance(other, &vm.ctx.int_type()) {
        return Ok(vm.ctx.not_implemented());
    }

    let zelf = BigInt::from_pyobj(zelf);
    let other = BigInt::from_pyobj(other);
    let result = zelf <= other;
    Ok(vm.ctx.new_bool(result))
}

fn int_gt(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.int_type())), (other, None)]
    );

    if !objtype::isinstance(other, &vm.ctx.int_type()) {
        return Ok(vm.ctx.not_implemented());
    }

    let zelf = BigInt::from_pyobj(zelf);
    let other = BigInt::from_pyobj(other);
    let result = zelf > other;
    Ok(vm.ctx.new_bool(result))
}

fn int_ge(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.int_type())), (other, None)]
    );

    if !objtype::isinstance(other, &vm.ctx.int_type()) {
        return Ok(vm.ctx.not_implemented());
    }

    let zelf = BigInt::from_pyobj(zelf);
    let other = BigInt::from_pyobj(other);
    let result = zelf >= other;
    Ok(vm.ctx.new_bool(result))
}

fn int_lshift(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(i, Some(vm.ctx.int_type())), (i2, None)]
    );

    if !objtype::isinstance(i2, &vm.ctx.int_type()) {
        return Err(vm.new_type_error(format!(
            "unsupported operand type(s) for << '{}' and '{}'",
            objtype::get_type_name(&i.typ()),
            objtype::get_type_name(&i2.typ())
        )));
    }

    if let Some(n_bits) = get_value(i2).to_usize() {
        return Ok(vm.ctx.new_int(get_value(i) << n_bits));
    }

    // i2 failed `to_usize()` conversion
    match get_value(i2) {
        ref v if *v < BigInt::zero() => Err(vm.new_value_error("negative shift count".to_string())),
        ref v if *v > BigInt::from(usize::max_value()) => {
            // TODO: raise OverflowError
            panic!("Failed converting {} to rust usize", get_value(i2));
        }
        _ => panic!("Failed converting {} to rust usize", get_value(i2)),
    }
}

fn int_rshift(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(i, Some(vm.ctx.int_type())), (i2, None)]
    );

    if !objtype::isinstance(i2, &vm.ctx.int_type()) {
        return Err(vm.new_type_error(format!(
            "unsupported operand type(s) for >> '{}' and '{}'",
            objtype::get_type_name(&i.typ()),
            objtype::get_type_name(&i2.typ())
        )));
    }

    if let Some(n_bits) = get_value(i2).to_usize() {
        return Ok(vm.ctx.new_int(get_value(i) >> n_bits));
    }

    // i2 failed `to_usize()` conversion
    match get_value(i2) {
        ref v if *v < BigInt::zero() => Err(vm.new_value_error("negative shift count".to_string())),
        ref v if *v > BigInt::from(usize::max_value()) => {
            // TODO: raise OverflowError
            panic!("Failed converting {} to rust usize", get_value(i2));
        }
        _ => panic!("Failed converting {} to rust usize", get_value(i2)),
    }
}

fn int_hash(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, Some(vm.ctx.int_type()))]);
    let value = BigInt::from_pyobj(zelf);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    let hash = hasher.finish();
    Ok(vm.ctx.new_int(hash))
}

fn int_abs(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(i, Some(vm.ctx.int_type()))]);
    Ok(vm.ctx.new_int(get_value(i).abs()))
}

fn int_add(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.int_type())), (other, None)]
    );
    if objtype::isinstance(other, &vm.ctx.int_type()) {
        Ok(vm.ctx.new_int(get_value(zelf) + get_value(other)))
    } else {
        Ok(vm.ctx.not_implemented())
    }
}

fn int_radd(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    int_add(vm, args)
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
        let (v1, v2) = (get_value(i), get_value(i2));

        if v2 != BigInt::zero() {
            Ok(vm.ctx.new_int(v1 / v2))
        } else {
            Err(vm.new_zero_division_error("integer floordiv by zero".to_string()))
        }
    } else {
        Ok(vm.ctx.not_implemented())
    }
}

fn int_round(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(i, Some(vm.ctx.int_type()))],
        optional = [(_precision, None)]
    );
    Ok(vm.ctx.new_int(get_value(i)))
}

fn int_pass_value(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(i, Some(vm.ctx.int_type()))]);
    Ok(vm.ctx.new_int(get_value(i)))
}

fn int_format(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [
            (i, Some(vm.ctx.int_type())),
            (format_spec, Some(vm.ctx.str_type()))
        ]
    );
    let string_value = objstr::get_value(format_spec);
    let format_spec = FormatSpec::parse(&string_value);
    let int_value = get_value(i);
    match format_spec.format_int(&int_value) {
        Ok(string) => Ok(vm.ctx.new_str(string)),
        Err(err) => Err(vm.new_value_error(err.to_string())),
    }
}

fn int_sub(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.int_type())), (other, None)]
    );
    if objtype::isinstance(other, &vm.ctx.int_type()) {
        Ok(vm.ctx.new_int(get_value(zelf) - get_value(other)))
    } else {
        Ok(vm.ctx.not_implemented())
    }
}

fn int_rsub(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.int_type())), (other, None)]
    );
    if objtype::isinstance(other, &vm.ctx.int_type()) {
        Ok(vm.ctx.new_int(get_value(other) - get_value(zelf)))
    } else {
        Ok(vm.ctx.not_implemented())
    }
}

fn int_mul(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.int_type())), (other, None)]
    );
    if objtype::isinstance(other, &vm.ctx.int_type()) {
        Ok(vm.ctx.new_int(get_value(zelf) * get_value(other)))
    } else {
        Ok(vm.ctx.not_implemented())
    }
}

fn int_rmul(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    int_mul(vm, args)
}

fn int_truediv(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.int_type())), (other, None)]
    );

    if objtype::isinstance(other, &vm.ctx.int_type()) {
        div_ints(vm, &get_value(zelf), &get_value(other))
    } else {
        Ok(vm.ctx.not_implemented())
    }
}

fn int_rtruediv(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.int_type())), (other, None)]
    );

    if objtype::isinstance(other, &vm.ctx.int_type()) {
        div_ints(vm, &get_value(other), &get_value(zelf))
    } else {
        Ok(vm.ctx.not_implemented())
    }
}

#[inline]
fn div_ints(vm: &mut VirtualMachine, i1: &BigInt, i2: &BigInt) -> PyResult {
    if i2.is_zero() {
        return Err(vm.new_zero_division_error("integer division by zero".to_string()));
    }

    if let (Some(f1), Some(f2)) = (i1.to_f64(), i2.to_f64()) {
        Ok(vm.ctx.new_float(f1 / f2))
    } else {
        let (quotient, mut rem) = i1.div_rem(i2);
        let mut divisor = i2.clone();

        if let Some(quotient) = quotient.to_f64() {
            let rem_part = loop {
                if rem.is_zero() {
                    break 0.0;
                } else if let (Some(rem), Some(divisor)) = (rem.to_f64(), divisor.to_f64()) {
                    break rem / divisor;
                } else {
                    // try with smaller numbers
                    rem /= 2;
                    divisor /= 2;
                }
            };

            Ok(vm.ctx.new_float(quotient + rem_part))
        } else {
            Err(vm.new_overflow_error("int too large to convert to float".to_string()))
        }
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
        let v2 = get_value(i2);

        if v2 != BigInt::zero() {
            Ok(vm.ctx.new_int(v1 % get_value(i2)))
        } else {
            Err(vm.new_zero_division_error("integer modulo by zero".to_string()))
        }
    } else {
        Ok(vm.ctx.not_implemented())
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
        Ok(vm.ctx.new_int(v1.pow(v2)))
    } else if objtype::isinstance(i2, &vm.ctx.float_type()) {
        let v2 = objfloat::get_value(i2);
        Ok(vm.ctx.new_float((v1.to_f64().unwrap()).powf(v2)))
    } else {
        Ok(vm.ctx.not_implemented())
    }
}

fn int_divmod(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(i, Some(vm.ctx.int_type())), (i2, None)]
    );

    if objtype::isinstance(i2, &vm.ctx.int_type()) {
        let v1 = get_value(i);
        let v2 = get_value(i2);

        if v2 != BigInt::zero() {
            let (r1, r2) = v1.div_rem(&v2);

            Ok(vm
                .ctx
                .new_tuple(vec![vm.ctx.new_int(r1), vm.ctx.new_int(r2)]))
        } else {
            Err(vm.new_zero_division_error("integer divmod by zero".to_string()))
        }
    } else {
        Ok(vm.ctx.not_implemented())
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
        Ok(vm.ctx.not_implemented())
    }
}

fn int_rxor(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(i, Some(vm.ctx.int_type())), (i2, None)]
    );

    if objtype::isinstance(i2, &vm.ctx.int_type()) {
        let right_val = get_value(i);
        let left_val = get_value(i2);

        Ok(vm.ctx.new_int(left_val ^ right_val))
    } else {
        Ok(vm.ctx.not_implemented())
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
        Ok(vm.ctx.not_implemented())
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
        Ok(vm.ctx.not_implemented())
    }
}

fn int_bit_length(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(i, Some(vm.ctx.int_type()))]);
    let v = get_value(i);
    let bits = v.bits();
    Ok(vm.ctx.new_int(bits))
}

fn int_conjugate(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(i, Some(vm.ctx.int_type()))]);
    let v = get_value(i);
    Ok(vm.ctx.new_int(v))
}

fn int_real(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, Some(vm.ctx.int_type()))]);
    let value = BigInt::from_pyobj(zelf);
    Ok(vm.ctx.new_int(value))
}

fn int_imag(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, Some(vm.ctx.int_type()))]);
    let value = BigInt::from(0);
    Ok(vm.ctx.new_int(value))
}

pub fn init(context: &PyContext) {
    let int_doc = "int(x=0) -> integer
int(x, base=10) -> integer

Convert a number or string to an integer, or return 0 if no arguments
are given.  If x is a number, return x.__int__().  For floating point
numbers, this truncates towards zero.

If x is not a number or if base is given, then x must be a string,
bytes, or bytearray instance representing an integer literal in the
given base.  The literal can be preceded by '+' or '-' and be surrounded
by whitespace.  The base defaults to 10.  Valid bases are 0 and 2-36.
Base 0 means to interpret the base from the string as an integer literal.
>>> int('0b100', base=0)
4";
    let int_type = &context.int_type;

    context.set_attr(&int_type, "__eq__", context.new_rustfunc(int_eq));
    context.set_attr(&int_type, "__ne__", context.new_rustfunc(int_ne));
    context.set_attr(&int_type, "__lt__", context.new_rustfunc(int_lt));
    context.set_attr(&int_type, "__le__", context.new_rustfunc(int_le));
    context.set_attr(&int_type, "__gt__", context.new_rustfunc(int_gt));
    context.set_attr(&int_type, "__ge__", context.new_rustfunc(int_ge));
    context.set_attr(&int_type, "__abs__", context.new_rustfunc(int_abs));
    context.set_attr(&int_type, "__add__", context.new_rustfunc(int_add));
    context.set_attr(&int_type, "__radd__", context.new_rustfunc(int_radd));
    context.set_attr(&int_type, "__and__", context.new_rustfunc(int_and));
    context.set_attr(&int_type, "__divmod__", context.new_rustfunc(int_divmod));
    context.set_attr(&int_type, "__float__", context.new_rustfunc(int_float));
    context.set_attr(&int_type, "__round__", context.new_rustfunc(int_round));
    context.set_attr(&int_type, "__ceil__", context.new_rustfunc(int_pass_value));
    context.set_attr(&int_type, "__floor__", context.new_rustfunc(int_pass_value));
    context.set_attr(&int_type, "__index__", context.new_rustfunc(int_pass_value));
    context.set_attr(&int_type, "__trunc__", context.new_rustfunc(int_pass_value));
    context.set_attr(&int_type, "__int__", context.new_rustfunc(int_pass_value));
    context.set_attr(
        &int_type,
        "__floordiv__",
        context.new_rustfunc(int_floordiv),
    );
    context.set_attr(&int_type, "__hash__", context.new_rustfunc(int_hash));
    context.set_attr(&int_type, "__lshift__", context.new_rustfunc(int_lshift));
    context.set_attr(&int_type, "__rshift__", context.new_rustfunc(int_rshift));
    context.set_attr(&int_type, "__new__", context.new_rustfunc(int_new));
    context.set_attr(&int_type, "__mod__", context.new_rustfunc(int_mod));
    context.set_attr(&int_type, "__mul__", context.new_rustfunc(int_mul));
    context.set_attr(&int_type, "__rmul__", context.new_rustfunc(int_rmul));
    context.set_attr(&int_type, "__neg__", context.new_rustfunc(int_neg));
    context.set_attr(&int_type, "__or__", context.new_rustfunc(int_or));
    context.set_attr(&int_type, "__pos__", context.new_rustfunc(int_pos));
    context.set_attr(&int_type, "__pow__", context.new_rustfunc(int_pow));
    context.set_attr(&int_type, "__repr__", context.new_rustfunc(int_repr));
    context.set_attr(&int_type, "__sub__", context.new_rustfunc(int_sub));
    context.set_attr(&int_type, "__rsub__", context.new_rustfunc(int_rsub));
    context.set_attr(&int_type, "__format__", context.new_rustfunc(int_format));
    context.set_attr(&int_type, "__truediv__", context.new_rustfunc(int_truediv));
    context.set_attr(
        &int_type,
        "__rtruediv__",
        context.new_rustfunc(int_rtruediv),
    );
    context.set_attr(&int_type, "__xor__", context.new_rustfunc(int_xor));
    context.set_attr(&int_type, "__rxor__", context.new_rustfunc(int_rxor));
    context.set_attr(&int_type, "__bool__", context.new_rustfunc(int_bool));
    context.set_attr(&int_type, "__invert__", context.new_rustfunc(int_invert));
    context.set_attr(
        &int_type,
        "bit_length",
        context.new_rustfunc(int_bit_length),
    );
    context.set_attr(&int_type, "__doc__", context.new_str(int_doc.to_string()));
    context.set_attr(&int_type, "conjugate", context.new_rustfunc(int_conjugate));
    context.set_attr(&int_type, "real", context.new_property(int_real));
    context.set_attr(&int_type, "imag", context.new_property(int_imag));
}
