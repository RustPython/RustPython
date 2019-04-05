use std::fmt;
use std::hash::{Hash, Hasher};

use num_bigint::{BigInt, ToBigInt};
use num_integer::Integer;
use num_traits::{Pow, Signed, ToPrimitive, Zero};

use crate::format::FormatSpec;
use crate::function::OptionalArg;
use crate::pyobject::{
    IntoPyObject, PyContext, PyObjectRef, PyRef, PyResult, PyValue, TryFromObject, TypeProtocol,
};
use crate::vm::VirtualMachine;

use super::objfloat::{self, PyFloat};
use super::objstr::{PyString, PyStringRef};
use super::objtype;
use crate::obj::objtype::PyClassRef;

#[derive(Debug)]
pub struct PyInt {
    value: BigInt,
}

impl fmt::Display for PyInt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        BigInt::fmt(&self.value, f)
    }
}

pub type PyIntRef = PyRef<PyInt>;

impl PyInt {
    pub fn new<T: Into<BigInt>>(i: T) -> Self {
        PyInt { value: i.into() }
    }

    pub fn as_bigint(&self) -> &BigInt {
        &self.value
    }
}

impl IntoPyObject for BigInt {
    fn into_pyobject(self, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.new_int(self))
    }
}

impl PyValue for PyInt {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.int_type()
    }
}

macro_rules! impl_into_pyobject_int {
    ($($t:ty)*) => {$(
        impl IntoPyObject for $t {
            fn into_pyobject(self, vm: &VirtualMachine) -> PyResult {
                Ok(vm.ctx.new_int(self))
            }
        }
    )*};
}

impl_into_pyobject_int!(isize i8 i16 i32 i64 usize u8 u16 u32 u64) ;

macro_rules! impl_try_from_object_int {
    ($(($t:ty, $to_prim:ident),)*) => {$(
        impl TryFromObject for $t {
            fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
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
    fn pass_value(self, _vm: &VirtualMachine) -> Self {
        self
    }

    fn eq(self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            vm.ctx.new_bool(self.value == *get_value(&other))
        } else {
            vm.ctx.not_implemented()
        }
    }

    fn ne(self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            vm.ctx.new_bool(self.value != *get_value(&other))
        } else {
            vm.ctx.not_implemented()
        }
    }

    fn lt(self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            vm.ctx.new_bool(self.value < *get_value(&other))
        } else {
            vm.ctx.not_implemented()
        }
    }

    fn le(self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            vm.ctx.new_bool(self.value <= *get_value(&other))
        } else {
            vm.ctx.not_implemented()
        }
    }

    fn gt(self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            vm.ctx.new_bool(self.value > *get_value(&other))
        } else {
            vm.ctx.not_implemented()
        }
    }

    fn ge(self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            vm.ctx.new_bool(self.value >= *get_value(&other))
        } else {
            vm.ctx.not_implemented()
        }
    }

    fn add(self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            vm.ctx.new_int((&self.value) + get_value(&other))
        } else {
            vm.ctx.not_implemented()
        }
    }

    fn sub(self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            vm.ctx.new_int((&self.value) - get_value(&other))
        } else {
            vm.ctx.not_implemented()
        }
    }

    fn rsub(self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            vm.ctx.new_int(get_value(&other) - (&self.value))
        } else {
            vm.ctx.not_implemented()
        }
    }

    fn mul(self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            vm.ctx.new_int((&self.value) * get_value(&other))
        } else {
            vm.ctx.not_implemented()
        }
    }

    fn truediv(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            div_ints(vm, &self.value, &get_value(&other))
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    fn rtruediv(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            div_ints(vm, &get_value(&other), &self.value)
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    fn floordiv(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
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

    fn lshift(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if !objtype::isinstance(&other, &vm.ctx.int_type()) {
            return Ok(vm.ctx.not_implemented());
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

    fn rshift(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if !objtype::isinstance(&other, &vm.ctx.int_type()) {
            return Ok(vm.ctx.not_implemented());
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

    fn xor(self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            vm.ctx.new_int((&self.value) ^ get_value(&other))
        } else {
            vm.ctx.not_implemented()
        }
    }

    fn rxor(self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            vm.ctx.new_int(get_value(&other) ^ (&self.value))
        } else {
            vm.ctx.not_implemented()
        }
    }

    fn or(self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            vm.ctx.new_int((&self.value) | get_value(&other))
        } else {
            vm.ctx.not_implemented()
        }
    }

    fn and(self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            let v2 = get_value(&other);
            vm.ctx.new_int((&self.value) & v2)
        } else {
            vm.ctx.not_implemented()
        }
    }

    fn pow(self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
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

    fn mod_(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
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

    fn divmod(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
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

    fn neg(self, _vm: &VirtualMachine) -> BigInt {
        -(&self.value)
    }

    fn hash(self, _vm: &VirtualMachine) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.value.hash(&mut hasher);
        hasher.finish()
    }

    fn abs(self, _vm: &VirtualMachine) -> BigInt {
        self.value.abs()
    }

    fn round(self, _precision: OptionalArg<PyObjectRef>, _vm: &VirtualMachine) -> Self {
        self
    }

    fn float(self, _vm: &VirtualMachine) -> f64 {
        self.value.to_f64().unwrap()
    }

    fn invert(self, _vm: &VirtualMachine) -> BigInt {
        !(&self.value)
    }

    fn repr(self, _vm: &VirtualMachine) -> String {
        self.value.to_string()
    }

    fn format(self, spec: PyStringRef, vm: &VirtualMachine) -> PyResult<String> {
        let format_spec = FormatSpec::parse(&spec.value);
        match format_spec.format_int(&self.value) {
            Ok(string) => Ok(string),
            Err(err) => Err(vm.new_value_error(err.to_string())),
        }
    }

    fn bool(self, _vm: &VirtualMachine) -> bool {
        !self.value.is_zero()
    }

    fn bit_length(self, _vm: &VirtualMachine) -> usize {
        self.value.bits()
    }

    fn imag(self, _vm: &VirtualMachine) -> usize {
        0
    }
}

#[derive(FromArgs)]
#[__inside_vm]
struct IntOptions {
    #[pyarg(positional_only, optional = true)]
    val_options: OptionalArg<PyObjectRef>,
    #[pyarg(positional_or_keyword, optional = true)]
    base: OptionalArg<u32>,
}

impl IntOptions {
    fn get_int_value(self, vm: &VirtualMachine) -> PyResult<BigInt> {
        if let OptionalArg::Present(val) = self.val_options {
            let base = if let OptionalArg::Present(base) = self.base {
                if !objtype::isinstance(&val, &vm.ctx.str_type) {
                    return Err(vm.new_type_error(
                        "int() can't convert non-string with explicit base".to_string(),
                    ));
                }
                base
            } else {
                10
            };
            to_int(vm, &val, base)
        } else if let OptionalArg::Present(_) = self.base {
            Err(vm.new_type_error("int() missing string argument".to_string()))
        } else {
            Ok(Zero::zero())
        }
    }
}

fn int_new(cls: PyClassRef, options: IntOptions, vm: &VirtualMachine) -> PyResult<PyIntRef> {
    PyInt::new(options.get_int_value(vm)?).into_ref_with_type(vm, cls)
}

// Casting function:
// TODO: this should just call `__int__` on the object
pub fn to_int(vm: &VirtualMachine, obj: &PyObjectRef, base: u32) -> PyResult<BigInt> {
    match_class!(obj.clone(),
        i @ PyInt => Ok(i.as_bigint().clone()),
        f @ PyFloat => Ok(f.to_f64().to_bigint().unwrap()),
        s @ PyString => {
            i32::from_str_radix(s.as_str(), base)
                .map(|i| BigInt::from(i))
                .map_err(|_|vm.new_value_error(format!(
                    "invalid literal for int() with base {}: '{}'",
                    base, s
                )))
        },
        obj => Err(vm.new_type_error(format!(
            "int() argument must be a string or a number, not '{}'",
            obj.class().name
        )))
    )
}

// Retrieve inner int value:
pub fn get_value(obj: &PyObjectRef) -> &BigInt {
    &obj.payload::<PyInt>().unwrap().value
}

#[inline]
fn div_ints(vm: &VirtualMachine, i1: &BigInt, i2: &BigInt) -> PyResult {
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
    extend_class!(context, int_type, {
        "__doc__" => context.new_str(int_doc.to_string()),
        "__eq__" => context.new_rustfunc(PyIntRef::eq),
        "__ne__" => context.new_rustfunc(PyIntRef::ne),
        "__lt__" => context.new_rustfunc(PyIntRef::lt),
        "__le__" => context.new_rustfunc(PyIntRef::le),
        "__gt__" => context.new_rustfunc(PyIntRef::gt),
        "__ge__" => context.new_rustfunc(PyIntRef::ge),
        "__abs__" => context.new_rustfunc(PyIntRef::abs),
        "__add__" => context.new_rustfunc(PyIntRef::add),
        "__radd__" => context.new_rustfunc(PyIntRef::add),
        "__and__" => context.new_rustfunc(PyIntRef::and),
        "__divmod__" => context.new_rustfunc(PyIntRef::divmod),
        "__float__" => context.new_rustfunc(PyIntRef::float),
        "__round__" => context.new_rustfunc(PyIntRef::round),
        "__ceil__" => context.new_rustfunc(PyIntRef::pass_value),
        "__floor__" => context.new_rustfunc(PyIntRef::pass_value),
        "__index__" => context.new_rustfunc(PyIntRef::pass_value),
        "__trunc__" => context.new_rustfunc(PyIntRef::pass_value),
        "__int__" => context.new_rustfunc(PyIntRef::pass_value),
        "__floordiv__" => context.new_rustfunc(PyIntRef::floordiv),
        "__hash__" => context.new_rustfunc(PyIntRef::hash),
        "__lshift__" => context.new_rustfunc(PyIntRef::lshift),
        "__rshift__" => context.new_rustfunc(PyIntRef::rshift),
        "__new__" => context.new_rustfunc(int_new),
        "__mod__" => context.new_rustfunc(PyIntRef::mod_),
        "__mul__" => context.new_rustfunc(PyIntRef::mul),
        "__rmul__" => context.new_rustfunc(PyIntRef::mul),
        "__or__" => context.new_rustfunc(PyIntRef::or),
        "__neg__" => context.new_rustfunc(PyIntRef::neg),
        "__pos__" => context.new_rustfunc(PyIntRef::pass_value),
        "__pow__" => context.new_rustfunc(PyIntRef::pow),
        "__repr__" => context.new_rustfunc(PyIntRef::repr),
        "__sub__" => context.new_rustfunc(PyIntRef::sub),
        "__rsub__" => context.new_rustfunc(PyIntRef::rsub),
        "__format__" => context.new_rustfunc(PyIntRef::format),
        "__truediv__" => context.new_rustfunc(PyIntRef::truediv),
        "__rtruediv__" => context.new_rustfunc(PyIntRef::rtruediv),
        "__xor__" => context.new_rustfunc(PyIntRef::xor),
        "__rxor__" => context.new_rustfunc(PyIntRef::rxor),
        "__bool__" => context.new_rustfunc(PyIntRef::bool),
        "__invert__" => context.new_rustfunc(PyIntRef::invert),
        "bit_length" => context.new_rustfunc(PyIntRef::bit_length),
        "conjugate" => context.new_rustfunc(PyIntRef::pass_value),
        "real" => context.new_property(PyIntRef::pass_value),
        "imag" => context.new_property(PyIntRef::imag)
    });
}
