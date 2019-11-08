use std::fmt;
use std::mem::size_of;
use std::str;

use num_bigint::{BigInt, Sign};
use num_integer::Integer;
use num_traits::{Num, One, Pow, Signed, ToPrimitive, Zero};

use super::objbool::IntoPyBool;
use super::objbyteinner::PyByteInner;
use super::objbytes::PyBytes;
use super::objfloat;
use super::objint;
use super::objstr::{PyString, PyStringRef};
use super::objtype::{self, PyClassRef};
use crate::format::FormatSpec;
use crate::function::{OptionalArg, PyFuncArgs};
use crate::pyhash;
use crate::pyobject::{
    IdProtocol, IntoPyObject, PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue,
    TryFromObject, TypeProtocol,
};
use crate::vm::VirtualMachine;

/// int(x=0) -> integer
/// int(x, base=10) -> integer
///
/// Convert a number or string to an integer, or return 0 if no arguments
/// are given.  If x is a number, return x.__int__().  For floating point
/// numbers, this truncates towards zero.
///
/// If x is not a number or if base is given, then x must be a string,
/// bytes, or bytearray instance representing an integer literal in the
/// given base.  The literal can be preceded by '+' or '-' and be surrounded
/// by whitespace.  The base defaults to 10.  Valid bases are 0 and 2-36.
/// Base 0 means to interpret the base from the string as an integer literal.
/// >>> int('0b100', base=0)
/// 4
#[pyclass]
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
                let int = PyIntRef::try_from_object(vm, obj)?;
                match int.value.$to_prim() {
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

#[allow(clippy::collapsible_if)]
fn inner_pow(int1: &PyInt, int2: &PyInt, vm: &VirtualMachine) -> PyResult {
    if int2.value.is_negative() {
        let v1 = int1.float(vm)?;
        let v2 = int2.float(vm)?;
        objfloat::float_pow(v1, v2, vm)
    } else {
        Ok(if let Some(v2) = int2.value.to_u64() {
            vm.ctx.new_int(int1.value.pow(v2))
        } else if int1.value.is_one() {
            vm.ctx.new_int(1)
        } else if int1.value.is_zero() {
            vm.ctx.new_int(0)
        } else if int1.value == BigInt::from(-1) {
            if int2.value.is_odd() {
                vm.ctx.new_int(-1)
            } else {
                vm.ctx.new_int(1)
            }
        } else {
            // missing feature: BigInt exp
            // practically, exp over u64 is not possible to calculate anyway
            vm.ctx.not_implemented()
        })
    }
}

fn inner_mod(int1: &PyInt, int2: &PyInt, vm: &VirtualMachine) -> PyResult {
    if int2.value.is_zero() {
        Err(vm.new_zero_division_error("integer modulo by zero".to_string()))
    } else {
        Ok(vm.ctx.new_int(int1.value.mod_floor(&int2.value)))
    }
}

fn inner_floordiv(int1: &PyInt, int2: &PyInt, vm: &VirtualMachine) -> PyResult {
    if int2.value.is_zero() {
        Err(vm.new_zero_division_error("integer division by zero".to_string()))
    } else {
        Ok(vm.ctx.new_int(int1.value.div_floor(&int2.value)))
    }
}

fn inner_divmod(int1: &PyInt, int2: &PyInt, vm: &VirtualMachine) -> PyResult {
    if int2.value.is_zero() {
        Err(vm.new_zero_division_error("integer division or modulo by zero".to_string()))
    } else {
        let (div, modulo) = int1.value.div_mod_floor(&int2.value);
        Ok(vm
            .ctx
            .new_tuple(vec![vm.ctx.new_int(div), vm.ctx.new_int(modulo)]))
    }
}

fn inner_lshift(int1: &PyInt, int2: &PyInt, vm: &VirtualMachine) -> PyResult {
    let n_bits = get_shift_amount(int2, vm)?;
    Ok(vm.ctx.new_int(&int1.value << n_bits))
}

fn inner_rshift(int1: &PyInt, int2: &PyInt, vm: &VirtualMachine) -> PyResult {
    let n_bits = get_shift_amount(int2, vm)?;
    Ok(vm.ctx.new_int(&int1.value >> n_bits))
}

#[pyimpl]
impl PyInt {
    #[pyslot(new)]
    fn tp_new(cls: PyClassRef, options: IntOptions, vm: &VirtualMachine) -> PyResult<PyIntRef> {
        PyInt::new(options.get_int_value(vm)?).into_ref_with_type(vm, cls)
    }

    #[pymethod(name = "__eq__")]
    fn eq(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            vm.ctx.new_bool(self.value == *get_value(&other))
        } else {
            vm.ctx.not_implemented()
        }
    }

    #[pymethod(name = "__ne__")]
    fn ne(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            vm.ctx.new_bool(self.value != *get_value(&other))
        } else {
            vm.ctx.not_implemented()
        }
    }

    #[pymethod(name = "__lt__")]
    fn lt(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            vm.ctx.new_bool(self.value < *get_value(&other))
        } else {
            vm.ctx.not_implemented()
        }
    }

    #[pymethod(name = "__le__")]
    fn le(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            vm.ctx.new_bool(self.value <= *get_value(&other))
        } else {
            vm.ctx.not_implemented()
        }
    }

    #[pymethod(name = "__gt__")]
    fn gt(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            vm.ctx.new_bool(self.value > *get_value(&other))
        } else {
            vm.ctx.not_implemented()
        }
    }

    #[pymethod(name = "__ge__")]
    fn ge(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            vm.ctx.new_bool(self.value >= *get_value(&other))
        } else {
            vm.ctx.not_implemented()
        }
    }

    #[pymethod(name = "__add__")]
    fn add(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            vm.ctx.new_int((&self.value) + get_value(&other))
        } else {
            vm.ctx.not_implemented()
        }
    }

    #[pymethod(name = "__radd__")]
    fn radd(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        self.add(other, vm)
    }

    #[pymethod(name = "__sub__")]
    fn sub(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            vm.ctx.new_int((&self.value) - get_value(&other))
        } else {
            vm.ctx.not_implemented()
        }
    }

    #[pymethod(name = "__rsub__")]
    fn rsub(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            vm.ctx.new_int(get_value(&other) - (&self.value))
        } else {
            vm.ctx.not_implemented()
        }
    }

    #[pymethod(name = "__mul__")]
    fn mul(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            vm.ctx.new_int((&self.value) * get_value(&other))
        } else {
            vm.ctx.not_implemented()
        }
    }

    #[pymethod(name = "__rmul__")]
    fn rmul(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        self.mul(other, vm)
    }

    #[pymethod(name = "__truediv__")]
    fn truediv(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            div_ints(vm, &self.value, &get_value(&other))
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    #[pymethod(name = "__rtruediv__")]
    fn rtruediv(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            div_ints(vm, &get_value(&other), &self.value)
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    #[pymethod(name = "__floordiv__")]
    fn floordiv(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            let other = get_py_int(&other);
            inner_floordiv(self, &other, &vm)
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    #[pymethod(name = "__rfloordiv__")]
    fn rfloordiv(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            let other = get_py_int(&other);
            inner_floordiv(&other, self, &vm)
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    #[pymethod(name = "__lshift__")]
    fn lshift(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if !objtype::isinstance(&other, &vm.ctx.int_type()) {
            return Ok(vm.ctx.not_implemented());
        }

        let other = get_py_int(&other);
        inner_lshift(self, other, vm)
    }

    #[pymethod(name = "__rlshift__")]
    fn rlshift(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if !objtype::isinstance(&other, &vm.ctx.int_type()) {
            return Ok(vm.ctx.not_implemented());
        }

        let other = get_py_int(&other);
        inner_lshift(other, self, vm)
    }

    #[pymethod(name = "__rshift__")]
    fn rshift(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if !objtype::isinstance(&other, &vm.ctx.int_type()) {
            return Ok(vm.ctx.not_implemented());
        }

        let other = get_py_int(&other);
        inner_rshift(self, other, vm)
    }

    #[pymethod(name = "__rrshift__")]
    fn rrshift(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if !objtype::isinstance(&other, &vm.ctx.int_type()) {
            return Ok(vm.ctx.not_implemented());
        }

        let other = get_py_int(&other);
        inner_rshift(other, self, vm)
    }

    #[pymethod(name = "__xor__")]
    pub fn xor(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            vm.ctx.new_int((&self.value) ^ get_value(&other))
        } else {
            vm.ctx.not_implemented()
        }
    }

    #[pymethod(name = "__rxor__")]
    fn rxor(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        self.xor(other, vm)
    }

    #[pymethod(name = "__or__")]
    pub fn or(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            vm.ctx.new_int((&self.value) | get_value(&other))
        } else {
            vm.ctx.not_implemented()
        }
    }

    #[pymethod(name = "__ror__")]
    fn ror(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        self.or(other, vm)
    }

    #[pymethod(name = "__and__")]
    pub fn and(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            let v2 = get_value(&other);
            vm.ctx.new_int((&self.value) & v2)
        } else {
            vm.ctx.not_implemented()
        }
    }

    #[pymethod(name = "__rand__")]
    fn rand(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        self.and(other, vm)
    }

    #[pymethod(name = "__pow__")]
    fn pow(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            let other = get_py_int(&other);
            inner_pow(self, &other, vm)
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    #[pymethod(name = "__rpow__")]
    fn rpow(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            let other = get_py_int(&other);
            inner_pow(&other, self, vm)
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    #[pymethod(name = "__mod__")]
    fn mod_(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            let other = get_py_int(&other);
            inner_mod(self, &other, vm)
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    #[pymethod(name = "__rmod__")]
    fn rmod(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            let other = get_py_int(&other);
            inner_mod(&other, self, vm)
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    #[pymethod(name = "__divmod__")]
    fn divmod(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            let other = get_py_int(&other);
            inner_divmod(self, &other, vm)
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    #[pymethod(name = "__rdivmod__")]
    fn rdivmod(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            let other = get_py_int(&other);
            inner_divmod(&other, self, vm)
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    #[pymethod(name = "__neg__")]
    fn neg(&self, _vm: &VirtualMachine) -> BigInt {
        -(&self.value)
    }

    #[pymethod(name = "__hash__")]
    pub fn hash(&self, _vm: &VirtualMachine) -> pyhash::PyHash {
        pyhash::hash_bigint(&self.value)
    }

    #[pymethod(name = "__abs__")]
    fn abs(&self, _vm: &VirtualMachine) -> BigInt {
        self.value.abs()
    }

    #[pymethod(name = "__round__")]
    fn round(
        zelf: PyRef<Self>,
        _precision: OptionalArg<PyObjectRef>,
        _vm: &VirtualMachine,
    ) -> PyResult<PyIntRef> {
        let _ndigits = match _precision {
            OptionalArg::Missing => None,
            OptionalArg::Present(ref value) => {
                if !_vm.get_none().is(value) {
                    if !objtype::isinstance(value, &_vm.ctx.int_type()) {
                        return Err(_vm.new_type_error(format!(
                            "'{}' object cannot be interpreted as an integer",
                            value.class().name
                        )));
                    };
                    // Only accept int type _ndigits
                    let _ndigits = objint::get_value(value);
                    Some(_ndigits)
                } else {
                    return Err(_vm.new_type_error(format!(
                        "'{}' object cannot be interpreted as an integer",
                        value.class().name
                    )));
                }
            }
        };
        Ok(zelf)
    }

    #[pymethod(name = "__int__")]
    fn int(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyIntRef {
        zelf
    }

    #[pymethod(name = "__pos__")]
    fn pos(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyIntRef {
        zelf
    }

    #[pymethod(name = "__float__")]
    fn float(&self, vm: &VirtualMachine) -> PyResult<f64> {
        self.value
            .to_f64()
            .ok_or_else(|| vm.new_overflow_error("int too large to convert to float".to_string()))
    }

    #[pymethod(name = "__trunc__")]
    fn trunc(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyIntRef {
        zelf
    }

    #[pymethod(name = "__floor__")]
    fn floor(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyIntRef {
        zelf
    }

    #[pymethod(name = "__ceil__")]
    fn ceil(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyIntRef {
        zelf
    }

    #[pymethod(name = "__index__")]
    fn index(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyIntRef {
        zelf
    }

    #[pymethod(name = "__invert__")]
    fn invert(&self, _vm: &VirtualMachine) -> BigInt {
        !(&self.value)
    }

    #[pymethod(name = "__repr__")]
    fn repr(&self, _vm: &VirtualMachine) -> String {
        self.value.to_string()
    }

    #[pymethod(name = "__format__")]
    fn format(&self, spec: PyStringRef, vm: &VirtualMachine) -> PyResult<String> {
        let format_spec = FormatSpec::parse(spec.as_str());
        match format_spec.format_int(&self.value) {
            Ok(string) => Ok(string),
            Err(err) => Err(vm.new_value_error(err.to_string())),
        }
    }

    #[pymethod(name = "__bool__")]
    fn bool(&self, _vm: &VirtualMachine) -> bool {
        !self.value.is_zero()
    }

    #[pymethod(name = "__sizeof__")]
    fn sizeof(&self, _vm: &VirtualMachine) -> usize {
        size_of::<Self>() + ((self.value.bits() + 7) & !7) / 8
    }

    #[pymethod]
    fn bit_length(&self, _vm: &VirtualMachine) -> usize {
        self.value.bits()
    }

    #[pymethod]
    fn conjugate(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyIntRef {
        zelf
    }

    #[pymethod]
    #[allow(clippy::match_bool)]
    fn from_bytes(args: IntFromByteArgs, vm: &VirtualMachine) -> PyResult<BigInt> {
        let signed = if let OptionalArg::Present(signed) = args.signed {
            signed.to_bool()
        } else {
            false
        };

        let x = match args.byteorder.as_str() {
            "big" => match signed {
                true => BigInt::from_signed_bytes_be(&args.bytes.elements),
                false => BigInt::from_bytes_be(Sign::Plus, &args.bytes.elements),
            },
            "little" => match signed {
                true => BigInt::from_signed_bytes_le(&args.bytes.elements),
                false => BigInt::from_bytes_le(Sign::Plus, &args.bytes.elements),
            },
            _ => {
                return Err(
                    vm.new_value_error("byteorder must be either 'little' or 'big'".to_string())
                )
            }
        };
        Ok(x)
    }

    #[pymethod]
    #[allow(clippy::match_bool)]
    fn to_bytes(&self, args: IntToByteArgs, vm: &VirtualMachine) -> PyResult<PyBytes> {
        let signed = if let OptionalArg::Present(signed) = args.signed {
            signed.to_bool()
        } else {
            false
        };

        let value = self.as_bigint();
        if value.sign() == Sign::Minus && !signed {
            return Err(vm.new_overflow_error("can't convert negative int to unsigned".to_string()));
        }

        let byte_len = if let Some(byte_len) = args.length.as_bigint().to_usize() {
            byte_len
        } else {
            return Err(
                vm.new_overflow_error("Python int too large to convert to C ssize_t".to_string())
            );
        };

        let mut origin_bytes = match args.byteorder.as_str() {
            "big" => match signed {
                true => value.to_signed_bytes_be(),
                false => value.to_bytes_be().1,
            },
            "little" => match signed {
                true => value.to_signed_bytes_le(),
                false => value.to_bytes_le().1,
            },
            _ => {
                return Err(
                    vm.new_value_error("byteorder must be either 'little' or 'big'".to_string())
                );
            }
        };

        let origin_len = origin_bytes.len();
        if origin_len > byte_len {
            return Err(vm.new_overflow_error("int too big to convert".to_string()));
        }

        let mut append_bytes = match value.sign() {
            Sign::Minus => vec![255u8; byte_len - origin_len],
            _ => vec![0u8; byte_len - origin_len],
        };

        let mut bytes = vec![];
        match args.byteorder.as_str() {
            "big" => {
                bytes = append_bytes;
                bytes.append(&mut origin_bytes);
            }
            "little" => {
                bytes = origin_bytes;
                bytes.append(&mut append_bytes);
            }
            _ => (),
        }
        Ok(PyBytes::new(bytes))
    }
    #[pyproperty]
    fn real(&self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_bigint(&self.value)
    }

    #[pyproperty]
    fn imag(&self, _vm: &VirtualMachine) -> usize {
        0
    }

    #[pyproperty]
    fn numerator(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyIntRef {
        zelf
    }

    #[pyproperty]
    fn denominator(&self, _vm: &VirtualMachine) -> usize {
        1
    }
}

#[derive(FromArgs)]
struct IntOptions {
    #[pyarg(positional_only, optional = true)]
    val_options: OptionalArg<PyObjectRef>,
    #[pyarg(positional_or_keyword, optional = true)]
    base: OptionalArg<PyIntRef>,
}

impl IntOptions {
    fn get_int_value(self, vm: &VirtualMachine) -> PyResult<BigInt> {
        if let OptionalArg::Present(val) = self.val_options {
            let base = if let OptionalArg::Present(base) = self.base {
                if !(objtype::isinstance(&val, &vm.ctx.str_type())
                    || objtype::isinstance(&val, &vm.ctx.bytes_type()))
                {
                    return Err(vm.new_type_error(
                        "int() can't convert non-string with explicit base".to_string(),
                    ));
                }
                base
            } else {
                PyInt::new(10).into_ref(vm)
            };
            to_int(vm, &val, base.as_bigint())
        } else if let OptionalArg::Present(_) = self.base {
            Err(vm.new_type_error("int() missing string argument".to_string()))
        } else {
            Ok(Zero::zero())
        }
    }
}

#[derive(FromArgs)]
struct IntFromByteArgs {
    #[pyarg(positional_or_keyword)]
    bytes: PyByteInner,
    #[pyarg(positional_or_keyword)]
    byteorder: PyStringRef,
    #[pyarg(keyword_only, optional = true)]
    signed: OptionalArg<IntoPyBool>,
}

#[derive(FromArgs)]
struct IntToByteArgs {
    #[pyarg(positional_or_keyword)]
    length: PyIntRef,
    #[pyarg(positional_or_keyword)]
    byteorder: PyStringRef,
    #[pyarg(keyword_only, optional = true)]
    signed: OptionalArg<IntoPyBool>,
}

// Casting function:
pub fn to_int(vm: &VirtualMachine, obj: &PyObjectRef, base: &BigInt) -> PyResult<BigInt> {
    let base_u32 = match base.to_u32() {
        Some(base_u32) => base_u32,
        None => {
            return Err(vm.new_value_error("int() base must be >= 2 and <= 36, or 0".to_string()))
        }
    };
    if base_u32 != 0 && (base_u32 < 2 || base_u32 > 36) {
        return Err(vm.new_value_error("int() base must be >= 2 and <= 36, or 0".to_string()));
    }

    match_class!(match obj.clone() {
        string @ PyString => {
            let s = string.as_str().trim();
            str_to_int(vm, s, base)
        }
        bytes @ PyBytes => {
            let bytes = bytes.get_value();
            let s = std::str::from_utf8(bytes)
                .map(|s| s.trim())
                .map_err(|e| vm.new_value_error(format!("utf8 decode error: {}", e)))?;
            str_to_int(vm, s, base)
        }
        obj => {
            let method = vm.get_method_or_type_error(obj.clone(), "__int__", || {
                format!(
                    "int() argument must be a string or a number, not '{}'",
                    obj.class().name
                )
            })?;
            let result = vm.invoke(&method, PyFuncArgs::default())?;
            match result.payload::<PyInt>() {
                Some(int_obj) => Ok(int_obj.as_bigint().clone()),
                None => Err(vm.new_type_error(format!(
                    "TypeError: __int__ returned non-int (type '{}')",
                    result.class().name
                ))),
            }
        }
    })
}

fn str_to_int(vm: &VirtualMachine, literal: &str, base: &BigInt) -> PyResult<BigInt> {
    let mut buf = validate_literal(vm, literal, base)?;
    let is_signed = buf.starts_with('+') || buf.starts_with('-');
    let radix_range = if is_signed { 1..3 } else { 0..2 };
    let radix_candidate = buf.get(radix_range.clone());

    let mut base_u32 = match base.to_u32() {
        Some(base_u32) => base_u32,
        None => return Err(invalid_literal(vm, literal, base)),
    };

    // try to find base
    if let Some(radix_candidate) = radix_candidate {
        if let Some(matched_radix) = detect_base(&radix_candidate) {
            if base_u32 == 0 || base_u32 == matched_radix {
                /* If base is 0 or equal radix number, it means radix is validate
                 * So change base to radix number and remove radix from literal
                 */
                base_u32 = matched_radix;
                buf.drain(radix_range);

                /* first underscore with radix is validate
                 * e.g : int(`0x_1`, base=0) = int('1', base=16)
                 */
                if buf.starts_with('_') {
                    buf.remove(0);
                }
            } else if (matched_radix == 2 && base_u32 < 12)
                || (matched_radix == 8 && base_u32 < 25)
                || (matched_radix == 16 && base_u32 < 34)
            {
                return Err(invalid_literal(vm, literal, base));
            }
        }
    }

    // base still not found, try to use default
    if base_u32 == 0 {
        if buf.starts_with('0') {
            return Err(invalid_literal(vm, literal, base));
        }

        base_u32 = 10;
    }

    BigInt::from_str_radix(&buf, base_u32).map_err(|_err| invalid_literal(vm, literal, base))
}

fn validate_literal(vm: &VirtualMachine, literal: &str, base: &BigInt) -> PyResult<String> {
    if literal.starts_with('_') || literal.ends_with('_') {
        return Err(invalid_literal(vm, literal, base));
    }

    let mut buf = String::with_capacity(literal.len());
    let mut last_tok = None;
    for c in literal.chars() {
        if !(c.is_ascii_alphanumeric() || c == '_' || c == '+' || c == '-') {
            return Err(invalid_literal(vm, literal, base));
        }

        if c == '_' && Some(c) == last_tok {
            return Err(invalid_literal(vm, literal, base));
        }

        last_tok = Some(c);
        buf.push(c);
    }

    Ok(buf)
}

fn detect_base(literal: &str) -> Option<u32> {
    match literal {
        "0x" | "0X" => Some(16),
        "0o" | "0O" => Some(8),
        "0b" | "0B" => Some(2),
        _ => None,
    }
}

fn invalid_literal(vm: &VirtualMachine, literal: &str, base: &BigInt) -> PyObjectRef {
    vm.new_value_error(format!(
        "invalid literal for int() with base {}: '{}'",
        base, literal
    ))
}

// Retrieve inner int value:
pub fn get_value(obj: &PyObjectRef) -> &BigInt {
    &get_py_int(obj).value
}

pub fn get_float_value(obj: &PyObjectRef, vm: &VirtualMachine) -> PyResult<f64> {
    get_py_int(obj).float(vm)
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

fn get_shift_amount(amount: &PyInt, vm: &VirtualMachine) -> PyResult<usize> {
    if let Some(n_bits) = amount.value.to_usize() {
        Ok(n_bits)
    } else {
        match &amount.value {
            v if *v < BigInt::zero() => Err(vm.new_value_error("negative shift count".to_string())),
            v if *v > BigInt::from(usize::max_value()) => {
                Err(vm.new_overflow_error("the number is too large to convert to int".to_string()))
            }
            _ => panic!("Failed converting {} to rust usize", amount.value),
        }
    }
}

fn get_py_int(obj: &PyObjectRef) -> &PyInt {
    &obj.payload::<PyInt>().unwrap()
}

pub fn init(context: &PyContext) {
    PyInt::extend_class(context, &context.types.int_type);
}
