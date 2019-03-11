use std::hash::{Hash, Hasher};

use num_bigint::{BigInt, ToBigInt};
use num_integer::Integer;
use num_traits::{Pow, Signed, ToPrimitive, Zero};

use crate::format::FormatSpec;
use crate::pyobject::{
    IntoPyObject, OptionalArg, PyContext, PyFuncArgs, PyObject, PyObjectRef, PyRef, PyResult,
    PyValue, TryFromObject, TypeProtocol,
};
use crate::vm::VirtualMachine;

use super::objfloat;
use super::objstr;
use super::objtype;

#[derive(Debug)]
pub struct PyInt {
    // TODO: shouldn't be public
    pub value: BigInt,
}

pub type PyIntRef = PyRef<PyInt>;

impl PyInt {
    pub fn new<T: ToBigInt>(i: T) -> Self {
        PyInt {
            // TODO: this .clone()s a BigInt, which is not what we want.
            value: i.to_bigint().unwrap(),
        }
    }
}

impl PyValue for PyInt {
    fn required_type(ctx: &PyContext) -> PyObjectRef {
        ctx.int_type()
    }
}

macro_rules! impl_into_pyobject_int {
    ($($t:ty)*) => {$(
        impl IntoPyObject for $t {
            fn into_pyobject(self, ctx: &PyContext) -> PyResult {
                Ok(ctx.new_int(self))
            }
        }
    )*};
}

impl_into_pyobject_int!(isize i8 i16 i32 i64 usize u8 u16 u32 u64) ;

macro_rules! impl_try_from_object_int {
    ($(($t:ty, $to_prim:ident),)*) => {$(
        impl TryFromObject for $t {
            fn try_from_object(vm: &mut VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
                match PyRef::<PyInt>::try_from_object(vm, obj)?.value.$to_prim() {
                    Some(value) => Ok(value),
                    None => Err(
                        vm.new_overflow_error(concat!(
                            "Int value cannot fit into Rust ",
                            stringify!($t)
                        ).to_string())
                    ),
                }
            }
        }
    )*};
}

impl_try_from_object_int!(
    (isize, to_isize),
    (i8, to_i8),
    (i16, to_i16),
    (i32, to_i32),
    (i64, to_i64),
    (usize, to_usize),
    (u8, to_u8),
    (u16, to_u16),
    (u32, to_u32),
    (u64, to_u64),
);

impl PyIntRef {
    fn pass_value(self, _vm: &mut VirtualMachine) -> Self {
        self
    }

    fn eq(self, other: PyObjectRef, vm: &mut VirtualMachine) -> PyObjectRef {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            vm.ctx.new_bool(self.value == *get_value(&other))
        } else {
            vm.ctx.not_implemented()
        }
    }

    fn ne(self, other: PyObjectRef, vm: &mut VirtualMachine) -> PyObjectRef {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            vm.ctx.new_bool(self.value != *get_value(&other))
        } else {
            vm.ctx.not_implemented()
        }
    }

    fn lt(self, other: PyObjectRef, vm: &mut VirtualMachine) -> PyObjectRef {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            vm.ctx.new_bool(self.value < *get_value(&other))
        } else {
            vm.ctx.not_implemented()
        }
    }

    fn le(self, other: PyObjectRef, vm: &mut VirtualMachine) -> PyObjectRef {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            vm.ctx.new_bool(self.value <= *get_value(&other))
        } else {
            vm.ctx.not_implemented()
        }
    }

    fn gt(self, other: PyObjectRef, vm: &mut VirtualMachine) -> PyObjectRef {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            vm.ctx.new_bool(self.value > *get_value(&other))
        } else {
            vm.ctx.not_implemented()
        }
    }

    fn ge(self, other: PyObjectRef, vm: &mut VirtualMachine) -> PyObjectRef {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            vm.ctx.new_bool(self.value >= *get_value(&other))
        } else {
            vm.ctx.not_implemented()
        }
    }

    fn add(self, other: PyObjectRef, vm: &mut VirtualMachine) -> PyObjectRef {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            vm.ctx.new_int((&self.value) + get_value(&other))
        } else {
            vm.ctx.not_implemented()
        }
    }

    fn sub(self, other: PyObjectRef, vm: &mut VirtualMachine) -> PyObjectRef {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            vm.ctx.new_int((&self.value) - get_value(&other))
        } else {
            vm.ctx.not_implemented()
        }
    }

    fn rsub(self, other: PyObjectRef, vm: &mut VirtualMachine) -> PyObjectRef {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            vm.ctx.new_int(get_value(&other) - (&self.value))
        } else {
            vm.ctx.not_implemented()
        }
    }

    fn mul(self, other: PyObjectRef, vm: &mut VirtualMachine) -> PyObjectRef {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            vm.ctx.new_int((&self.value) * get_value(&other))
        } else {
            vm.ctx.not_implemented()
        }
    }

    fn truediv(self, other: PyObjectRef, vm: &mut VirtualMachine) -> PyResult {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            div_ints(vm, &self.value, &get_value(&other))
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    fn rtruediv(self, other: PyObjectRef, vm: &mut VirtualMachine) -> PyResult {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            div_ints(vm, &get_value(&other), &self.value)
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    fn floordiv(self, other: PyObjectRef, vm: &mut VirtualMachine) -> PyResult {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            let v2 = get_value(&other);
            if *v2 != BigInt::zero() {
                Ok(vm.ctx.new_int((&self.value) / v2))
            } else {
                Err(vm.new_zero_division_error("integer floordiv by zero".to_string()))
            }
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    fn lshift(self, other: PyObjectRef, vm: &mut VirtualMachine) -> PyResult {
        if !objtype::isinstance(&other, &vm.ctx.int_type()) {
            return Err(vm.new_type_error(format!(
                "unsupported operand type(s) for << '{}' and '{}'",
                objtype::get_type_name(&self.as_object().typ()),
                objtype::get_type_name(&other.typ())
            )));
        }

        if let Some(n_bits) = get_value(&other).to_usize() {
            return Ok(vm.ctx.new_int((&self.value) << n_bits));
        }

        // i2 failed `to_usize()` conversion
        match get_value(&other) {
            v if *v < BigInt::zero() => Err(vm.new_value_error("negative shift count".to_string())),
            v if *v > BigInt::from(usize::max_value()) => {
                Err(vm.new_overflow_error("the number is too large to convert to int".to_string()))
            }
            _ => panic!("Failed converting {} to rust usize", get_value(&other)),
        }
    }

    fn rshift(self, other: PyObjectRef, vm: &mut VirtualMachine) -> PyResult {
        if !objtype::isinstance(&other, &vm.ctx.int_type()) {
            return Err(vm.new_type_error(format!(
                "unsupported operand type(s) for >> '{}' and '{}'",
                objtype::get_type_name(&self.as_object().typ()),
                objtype::get_type_name(&other.typ())
            )));
        }

        if let Some(n_bits) = get_value(&other).to_usize() {
            return Ok(vm.ctx.new_int((&self.value) >> n_bits));
        }

        // i2 failed `to_usize()` conversion
        match get_value(&other) {
            v if *v < BigInt::zero() => Err(vm.new_value_error("negative shift count".to_string())),
            v if *v > BigInt::from(usize::max_value()) => {
                Err(vm.new_overflow_error("the number is too large to convert to int".to_string()))
            }
            _ => panic!("Failed converting {} to rust usize", get_value(&other)),
        }
    }

    fn xor(self, other: PyObjectRef, vm: &mut VirtualMachine) -> PyObjectRef {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            vm.ctx.new_int((&self.value) ^ get_value(&other))
        } else {
            vm.ctx.not_implemented()
        }
    }

    fn rxor(self, other: PyObjectRef, vm: &mut VirtualMachine) -> PyObjectRef {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            vm.ctx.new_int(get_value(&other) ^ (&self.value))
        } else {
            vm.ctx.not_implemented()
        }
    }

    fn or(self, other: PyObjectRef, vm: &mut VirtualMachine) -> PyObjectRef {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            vm.ctx.new_int((&self.value) | get_value(&other))
        } else {
            vm.ctx.not_implemented()
        }
    }

    fn and(self, other: PyObjectRef, vm: &mut VirtualMachine) -> PyObjectRef {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            let v2 = get_value(&other);
            vm.ctx.new_int((&self.value) & v2)
        } else {
            vm.ctx.not_implemented()
        }
    }

    fn pow(self, other: PyObjectRef, vm: &mut VirtualMachine) -> PyObjectRef {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            let v2 = get_value(&other).to_u32().unwrap();
            vm.ctx.new_int(self.value.pow(v2))
        } else if objtype::isinstance(&other, &vm.ctx.float_type()) {
            let v2 = objfloat::get_value(&other);
            vm.ctx.new_float((self.value.to_f64().unwrap()).powf(v2))
        } else {
            vm.ctx.not_implemented()
        }
    }

    fn mod_(self, other: PyObjectRef, vm: &mut VirtualMachine) -> PyResult {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            let v2 = get_value(&other);
            if *v2 != BigInt::zero() {
                Ok(vm.ctx.new_int((&self.value) % v2))
            } else {
                Err(vm.new_zero_division_error("integer modulo by zero".to_string()))
            }
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    fn divmod(self, other: PyObjectRef, vm: &mut VirtualMachine) -> PyResult {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            let v2 = get_value(&other);
            if *v2 != BigInt::zero() {
                let (r1, r2) = self.value.div_rem(v2);
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

    fn neg(self, vm: &mut VirtualMachine) -> PyObjectRef {
        vm.ctx.new_int(-(&self.value))
    }

    fn hash(self, _vm: &mut VirtualMachine) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.value.hash(&mut hasher);
        hasher.finish()
    }

    fn abs(self, vm: &mut VirtualMachine) -> PyObjectRef {
        vm.ctx.new_int(self.value.abs())
    }

    fn round(self, _precision: OptionalArg<PyObjectRef>, _vm: &mut VirtualMachine) -> Self {
        self
    }

    fn float(self, _vm: &mut VirtualMachine) -> f64 {
        self.value.to_f64().unwrap()
    }

    fn invert(self, vm: &mut VirtualMachine) -> PyObjectRef {
        vm.ctx.new_int(!(&self.value))
    }

    fn repr(self, _vm: &mut VirtualMachine) -> String {
        self.value.to_string()
    }

    fn format(self, spec: PyRef<objstr::PyString>, vm: &mut VirtualMachine) -> PyResult<String> {
        let format_spec = FormatSpec::parse(&spec.value);
        match format_spec.format_int(&self.value) {
            Ok(string) => Ok(string),
            Err(err) => Err(vm.new_value_error(err.to_string())),
        }
    }

    fn bool(self, _vm: &mut VirtualMachine) -> bool {
        !self.value.is_zero()
    }

    fn bit_length(self, _vm: &mut VirtualMachine) -> usize {
        self.value.bits()
    }

    fn imag(self, _vm: &mut VirtualMachine) -> usize {
        0
    }
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

    let base = match args.get_optional_kwarg("base") {
        Some(argument) => get_value(&argument).to_u32().unwrap(),
        None => 10,
    };
    let val = match val_option {
        Some(val) => to_int(vm, val, base)?,
        None => Zero::zero(),
    };
    Ok(PyObject::new(PyInt::new(val), cls.clone()))
}

// Casting function:
pub fn to_int(vm: &mut VirtualMachine, obj: &PyObjectRef, base: u32) -> PyResult<BigInt> {
    let val = if objtype::isinstance(obj, &vm.ctx.int_type()) {
        get_value(obj).clone()
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
pub fn get_value(obj: &PyObjectRef) -> &BigInt {
    &obj.payload::<PyInt>().unwrap().value
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

#[rustfmt::skip] // to avoid line splitting
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

    context.set_attr(&int_type, "__doc__", context.new_str(int_doc.to_string()));
    context.set_attr(&int_type, "__eq__", context.new_rustfunc(PyIntRef::eq));
    context.set_attr(&int_type, "__ne__", context.new_rustfunc(PyIntRef::ne));
    context.set_attr(&int_type, "__lt__", context.new_rustfunc(PyIntRef::lt));
    context.set_attr(&int_type, "__le__", context.new_rustfunc(PyIntRef::le));
    context.set_attr(&int_type, "__gt__", context.new_rustfunc(PyIntRef::gt));
    context.set_attr(&int_type, "__ge__", context.new_rustfunc(PyIntRef::ge));
    context.set_attr(&int_type, "__abs__", context.new_rustfunc(PyIntRef::abs));
    context.set_attr(&int_type, "__add__", context.new_rustfunc(PyIntRef::add));
    context.set_attr(&int_type, "__radd__", context.new_rustfunc(PyIntRef::add));
    context.set_attr(&int_type, "__and__", context.new_rustfunc(PyIntRef::and));
    context.set_attr(&int_type, "__divmod__", context.new_rustfunc(PyIntRef::divmod));
    context.set_attr(&int_type, "__float__", context.new_rustfunc(PyIntRef::float));
    context.set_attr(&int_type, "__round__", context.new_rustfunc(PyIntRef::round));
    context.set_attr(&int_type, "__ceil__", context.new_rustfunc(PyIntRef::pass_value));
    context.set_attr(&int_type, "__floor__", context.new_rustfunc(PyIntRef::pass_value));
    context.set_attr(&int_type, "__index__", context.new_rustfunc(PyIntRef::pass_value));
    context.set_attr(&int_type, "__trunc__", context.new_rustfunc(PyIntRef::pass_value));
    context.set_attr(&int_type, "__int__", context.new_rustfunc(PyIntRef::pass_value));
    context.set_attr(&int_type, "__floordiv__", context.new_rustfunc(PyIntRef::floordiv));
    context.set_attr(&int_type, "__hash__", context.new_rustfunc(PyIntRef::hash));
    context.set_attr(&int_type, "__lshift__", context.new_rustfunc(PyIntRef::lshift));
    context.set_attr(&int_type, "__rshift__", context.new_rustfunc(PyIntRef::rshift));
    context.set_attr(&int_type, "__new__", context.new_rustfunc(int_new));
    context.set_attr(&int_type, "__mod__", context.new_rustfunc(PyIntRef::mod_));
    context.set_attr(&int_type, "__mul__", context.new_rustfunc(PyIntRef::mul));
    context.set_attr(&int_type, "__rmul__", context.new_rustfunc(PyIntRef::mul));
    context.set_attr(&int_type, "__or__", context.new_rustfunc(PyIntRef::or));
    context.set_attr(&int_type, "__neg__", context.new_rustfunc(PyIntRef::neg));
    context.set_attr(&int_type, "__pos__", context.new_rustfunc(PyIntRef::pass_value));
    context.set_attr(&int_type, "__pow__", context.new_rustfunc(PyIntRef::pow));
    context.set_attr(&int_type, "__repr__", context.new_rustfunc(PyIntRef::repr));
    context.set_attr(&int_type, "__sub__", context.new_rustfunc(PyIntRef::sub));
    context.set_attr(&int_type, "__rsub__", context.new_rustfunc(PyIntRef::rsub));
    context.set_attr(&int_type, "__format__", context.new_rustfunc(PyIntRef::format));
    context.set_attr(&int_type, "__truediv__", context.new_rustfunc(PyIntRef::truediv));
    context.set_attr(&int_type, "__rtruediv__", context.new_rustfunc(PyIntRef::rtruediv));
    context.set_attr(&int_type, "__xor__", context.new_rustfunc(PyIntRef::xor));
    context.set_attr(&int_type, "__rxor__", context.new_rustfunc(PyIntRef::rxor));
    context.set_attr(&int_type, "__bool__", context.new_rustfunc(PyIntRef::bool));
    context.set_attr(&int_type, "__invert__", context.new_rustfunc(PyIntRef::invert));
    context.set_attr(&int_type, "bit_length", context.new_rustfunc(PyIntRef::bit_length));
    context.set_attr(&int_type, "conjugate", context.new_rustfunc(PyIntRef::pass_value));
    context.set_attr(&int_type, "real", context.new_property(PyIntRef::pass_value));
    context.set_attr(&int_type, "imag", context.new_property(PyIntRef::imag));
}
