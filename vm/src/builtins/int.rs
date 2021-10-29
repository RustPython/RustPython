use super::{float, PyByteArray, PyBytes, PyStr, PyStrRef, PyTypeRef};
use crate::{
    bytesinner::PyBytesInner,
    common::hash,
    format::FormatSpec,
    function::{ArgIntoBool, IntoPyObject, IntoPyResult, OptionalArg, OptionalOption},
    try_value_from_borrowed_object,
    types::{Comparable, Constructor, Hashable, PyComparisonOp},
    IdProtocol, PyArithmeticValue, PyClassImpl, PyComparisonValue, PyContext, PyObject,
    PyObjectRef, PyRef, PyResult, PyValue, TryFromBorrowedObject, TypeProtocol, VirtualMachine,
};
use bstr::ByteSlice;
use num_bigint::{BigInt, BigUint, Sign};
use num_integer::Integer;
use num_traits::{One, Pow, PrimInt, Signed, ToPrimitive, Zero};
use std::fmt;
use std::mem::size_of;

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
#[pyclass(module = false, name = "int")]
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

impl<T> From<T> for PyInt
where
    T: Into<BigInt>,
{
    fn from(v: T) -> Self {
        Self { value: v.into() }
    }
}

impl PyValue for PyInt {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.int_type
    }

    fn into_object(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_int(self.value).into()
    }

    fn special_retrieve(vm: &VirtualMachine, obj: &PyObject) -> Option<PyResult<PyRef<Self>>> {
        Some(vm.to_index(obj))
    }
}

macro_rules! impl_into_pyobject_int {
    ($($t:ty)*) => {$(
        impl IntoPyObject for $t {
            fn into_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
                vm.ctx.new_int(self).into()
            }
        }
    )*};
}

impl_into_pyobject_int!(isize i8 i16 i32 i64 i128 usize u8 u16 u32 u64 u128 BigInt);

macro_rules! impl_try_from_object_int {
    ($(($t:ty, $to_prim:ident),)*) => {$(
        impl TryFromBorrowedObject for $t {
            fn try_from_borrowed_object(vm: &VirtualMachine, obj: &PyObject) -> PyResult<Self> {
                try_value_from_borrowed_object(vm, obj, |int: &PyInt| {
                    int.try_to_primitive(vm)
                })
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
    (i128, to_i128),
    (usize, to_usize),
    (u8, to_u8),
    (u16, to_u16),
    (u32, to_u32),
    (u64, to_u64),
    (u128, to_u128),
);

fn inner_pow(int1: &BigInt, int2: &BigInt, vm: &VirtualMachine) -> PyResult {
    if int2.is_negative() {
        let v1 = try_to_float(int1, vm)?;
        let v2 = try_to_float(int2, vm)?;
        float::float_pow(v1, v2, vm)
    } else {
        let value = if let Some(v2) = int2.to_u64() {
            return Ok(vm.ctx.new_int(Pow::pow(int1, v2)).into());
        } else if int1.is_one() {
            1
        } else if int1.is_zero() {
            0
        } else if int1 == &BigInt::from(-1) {
            if int2.is_odd() {
                -1
            } else {
                1
            }
        } else {
            // missing feature: BigInt exp
            // practically, exp over u64 is not possible to calculate anyway
            return Ok(vm.ctx.not_implemented());
        };
        Ok(vm.ctx.new_int(value).into())
    }
}

fn inner_mod(int1: &BigInt, int2: &BigInt, vm: &VirtualMachine) -> PyResult {
    if int2.is_zero() {
        Err(vm.new_zero_division_error("integer modulo by zero".to_owned()))
    } else {
        Ok(vm.ctx.new_int(int1.mod_floor(int2)).into())
    }
}

fn inner_floordiv(int1: &BigInt, int2: &BigInt, vm: &VirtualMachine) -> PyResult {
    if int2.is_zero() {
        Err(vm.new_zero_division_error("integer division by zero".to_owned()))
    } else {
        Ok(vm.ctx.new_int(int1.div_floor(int2)).into())
    }
}

fn inner_divmod(int1: &BigInt, int2: &BigInt, vm: &VirtualMachine) -> PyResult {
    if int2.is_zero() {
        return Err(vm.new_zero_division_error("integer division or modulo by zero".to_owned()));
    }
    let (div, modulo) = int1.div_mod_floor(int2);
    Ok(vm.new_tuple((div, modulo)).into())
}

fn inner_shift<F>(int1: &BigInt, int2: &BigInt, shift_op: F, vm: &VirtualMachine) -> PyResult
where
    F: Fn(&BigInt, usize) -> BigInt,
{
    if int2.is_negative() {
        Err(vm.new_value_error("negative shift count".to_owned()))
    } else if int1.is_zero() {
        Ok(vm.ctx.new_int(0).into())
    } else {
        let int2 = int2.to_usize().ok_or_else(|| {
            vm.new_overflow_error("the number is too large to convert to int".to_owned())
        })?;
        Ok(vm.ctx.new_int(shift_op(int1, int2)).into())
    }
}

#[inline]
fn inner_truediv(i1: &BigInt, i2: &BigInt, vm: &VirtualMachine) -> PyResult {
    if i2.is_zero() {
        return Err(vm.new_zero_division_error("integer division by zero".to_owned()));
    }

    let value = if let (Some(f1), Some(f2)) = (i2f(i1), i2f(i2)) {
        f1 / f2
    } else {
        let (quotient, mut rem) = i1.div_rem(i2);
        let mut divisor = i2.clone();

        if let Some(quotient) = i2f(&quotient) {
            let rem_part = loop {
                if rem.is_zero() {
                    break 0.0;
                } else if let (Some(rem), Some(divisor)) = (i2f(&rem), i2f(&divisor)) {
                    break rem / divisor;
                } else {
                    // try with smaller numbers
                    rem /= 2;
                    divisor /= 2;
                }
            };

            quotient + rem_part
        } else {
            return Err(vm.new_overflow_error("int too large to convert to float".to_owned()));
        }
    };
    Ok(vm.ctx.new_float(value).into())
}

impl Constructor for PyInt {
    type Args = IntOptions;

    fn py_new(cls: PyTypeRef, options: Self::Args, vm: &VirtualMachine) -> PyResult {
        let value = if let OptionalArg::Present(val) = options.val_options {
            if let OptionalArg::Present(base) = options.base {
                let base = vm
                    .to_index(&base)?
                    .as_bigint()
                    .to_u32()
                    .filter(|&v| v == 0 || (2..=36).contains(&v))
                    .ok_or_else(|| {
                        vm.new_value_error("int() base must be >= 2 and <= 36, or 0".to_owned())
                    })?;
                try_int_radix(&val, base, vm)
            } else {
                let val = if cls.is(&vm.ctx.types.int_type) {
                    match val.downcast_exact::<PyInt>(vm) {
                        Ok(i) => {
                            return Ok(i.into_pyobject(vm));
                        }
                        Err(val) => val,
                    }
                } else {
                    val
                };

                try_int(&val, vm)
            }
        } else if let OptionalArg::Present(_) = options.base {
            Err(vm.new_type_error("int() missing string argument".to_owned()))
        } else {
            Ok(Zero::zero())
        }?;

        Self::with_value(cls, value, vm).into_pyresult(vm)
    }
}

impl PyInt {
    fn with_value<T>(cls: PyTypeRef, value: T, vm: &VirtualMachine) -> PyResult<PyRef<Self>>
    where
        T: Into<BigInt> + ToPrimitive,
    {
        if cls.is(&vm.ctx.types.int_type) {
            Ok(vm.ctx.new_int(value))
        } else if cls.is(&vm.ctx.types.bool_type) {
            Ok(vm.ctx.new_bool(!value.into().eq(&BigInt::zero())))
        } else {
            PyInt::from(value).into_ref_with_type(vm, cls)
        }
    }

    pub fn as_bigint(&self) -> &BigInt {
        &self.value
    }

    // _PyLong_AsUnsignedLongMask
    pub fn as_u32_mask(&self) -> u32 {
        let v = self.as_bigint();
        v.to_u32()
            .or_else(|| v.to_i32().map(|i| i as u32))
            .unwrap_or_else(|| {
                let mut out = 0u32;
                for digit in v.iter_u32_digits() {
                    out = out.wrapping_shl(32) | digit;
                }
                match v.sign() {
                    num_bigint::Sign::Minus => out * -1i32 as u32,
                    _ => out,
                }
            })
    }

    pub fn try_to_primitive<'a, I>(&'a self, vm: &VirtualMachine) -> PyResult<I>
    where
        I: PrimInt + TryFrom<&'a BigInt>,
    {
        I::try_from(self.as_bigint()).map_err(|_| {
            vm.new_overflow_error(format!(
                "Python int too large to convert to Rust {}",
                std::any::type_name::<I>()
            ))
        })
    }

    #[inline]
    fn int_op<F>(&self, other: PyObjectRef, op: F, vm: &VirtualMachine) -> PyArithmeticValue<BigInt>
    where
        F: Fn(&BigInt, &BigInt) -> BigInt,
    {
        let r = other
            .payload_if_subclass::<PyInt>(vm)
            .map(|other| op(&self.value, &other.value));
        PyArithmeticValue::from_option(r)
    }

    #[inline]
    fn general_op<F>(&self, other: PyObjectRef, op: F, vm: &VirtualMachine) -> PyResult
    where
        F: Fn(&BigInt, &BigInt) -> PyResult,
    {
        if let Some(other) = other.payload_if_subclass::<PyInt>(vm) {
            op(&self.value, &other.value)
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }
}

#[pyimpl(flags(BASETYPE), with(Comparable, Hashable, Constructor))]
impl PyInt {
    #[pymethod(name = "__radd__")]
    #[pymethod(magic)]
    fn add(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyArithmeticValue<BigInt> {
        self.int_op(other, |a, b| a + b, vm)
    }

    #[pymethod(magic)]
    fn sub(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyArithmeticValue<BigInt> {
        self.int_op(other, |a, b| a - b, vm)
    }

    #[pymethod(magic)]
    fn rsub(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyArithmeticValue<BigInt> {
        self.int_op(other, |a, b| b - a, vm)
    }

    #[pymethod(name = "__rmul__")]
    #[pymethod(magic)]
    fn mul(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyArithmeticValue<BigInt> {
        self.int_op(other, |a, b| a * b, vm)
    }

    #[pymethod(magic)]
    fn truediv(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.general_op(other, |a, b| inner_truediv(a, b, vm), vm)
    }

    #[pymethod(magic)]
    fn rtruediv(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.general_op(other, |a, b| inner_truediv(b, a, vm), vm)
    }

    #[pymethod(magic)]
    fn floordiv(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.general_op(other, |a, b| inner_floordiv(a, b, vm), vm)
    }

    #[pymethod(magic)]
    fn rfloordiv(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.general_op(other, |a, b| inner_floordiv(b, a, vm), vm)
    }

    #[pymethod(magic)]
    fn lshift(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.general_op(other, |a, b| inner_shift(a, b, |a, b| a << b, vm), vm)
    }

    #[pymethod(magic)]
    fn rlshift(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.general_op(other, |a, b| inner_shift(b, a, |a, b| a << b, vm), vm)
    }

    #[pymethod(magic)]
    fn rshift(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.general_op(other, |a, b| inner_shift(a, b, |a, b| a >> b, vm), vm)
    }

    #[pymethod(magic)]
    fn rrshift(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.general_op(other, |a, b| inner_shift(b, a, |a, b| a >> b, vm), vm)
    }

    #[pymethod(name = "__rxor__")]
    #[pymethod(magic)]
    pub fn xor(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyArithmeticValue<BigInt> {
        self.int_op(other, |a, b| a ^ b, vm)
    }

    #[pymethod(name = "__ror__")]
    #[pymethod(magic)]
    pub fn or(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyArithmeticValue<BigInt> {
        self.int_op(other, |a, b| a | b, vm)
    }

    #[pymethod(name = "__rand__")]
    #[pymethod(magic)]
    pub fn and(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyArithmeticValue<BigInt> {
        self.int_op(other, |a, b| a & b, vm)
    }

    #[pymethod(magic)]
    fn pow(
        &self,
        other: PyObjectRef,
        mod_val: OptionalOption<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        match mod_val.flatten() {
            Some(int_ref) => {
                let int = match int_ref.payload_if_subclass::<PyInt>(vm) {
                    Some(val) => val,
                    None => return Ok(vm.ctx.not_implemented()),
                };

                let modulus = int.as_bigint();
                if modulus.is_zero() {
                    return Err(vm.new_value_error("pow() 3rd argument cannot be 0".to_owned()));
                }
                self.general_op(
                    other,
                    |a, b| {
                        let i = if b.is_negative() {
                            // modular multiplicative inverse
                            // based on rust-num/num-integer#10, should hopefully be published soon
                            fn normalize(a: BigInt, n: &BigInt) -> BigInt {
                                let a = a % n;
                                if a.is_negative() {
                                    a + n
                                } else {
                                    a
                                }
                            }
                            fn inverse(a: BigInt, n: &BigInt) -> Option<BigInt> {
                                use num_integer::*;
                                let ExtendedGcd { gcd, x: c, .. } = a.extended_gcd(n);
                                if gcd.is_one() {
                                    Some(normalize(c, n))
                                } else {
                                    None
                                }
                            }
                            let a = inverse(a % modulus, modulus).ok_or_else(|| {
                                vm.new_value_error(
                                    "base is not invertible for the given modulus".to_owned(),
                                )
                            })?;
                            let b = -b;
                            a.modpow(&b, modulus)
                        } else {
                            a.modpow(b, modulus)
                        };
                        Ok(vm.ctx.new_int(i).into())
                    },
                    vm,
                )
            }
            None => self.general_op(other, |a, b| inner_pow(a, b, vm), vm),
        }
    }

    #[pymethod(magic)]
    fn rpow(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.general_op(other, |a, b| inner_pow(b, a, vm), vm)
    }

    #[pymethod(name = "__mod__")]
    fn mod_(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.general_op(other, |a, b| inner_mod(a, b, vm), vm)
    }

    #[pymethod(magic)]
    fn rmod(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.general_op(other, |a, b| inner_mod(b, a, vm), vm)
    }

    #[pymethod(magic)]
    fn divmod(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.general_op(other, |a, b| inner_divmod(a, b, vm), vm)
    }

    #[pymethod(magic)]
    fn rdivmod(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.general_op(other, |a, b| inner_divmod(b, a, vm), vm)
    }

    #[pymethod(magic)]
    fn neg(&self) -> BigInt {
        -(&self.value)
    }

    #[pymethod(magic)]
    fn abs(&self) -> BigInt {
        self.value.abs()
    }

    #[pymethod(magic)]
    fn round(
        zelf: PyRef<Self>,
        precision: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<Self>> {
        match precision {
            OptionalArg::Missing => (),
            OptionalArg::Present(ref value) => {
                if !vm.is_none(value) {
                    // Only accept int type ndigits
                    let _ndigits = value.payload_if_subclass::<PyInt>(vm).ok_or_else(|| {
                        vm.new_type_error(format!(
                            "'{}' object cannot be interpreted as an integer",
                            value.class().name()
                        ))
                    })?;
                } else {
                    return Err(vm.new_type_error(format!(
                        "'{}' object cannot be interpreted as an integer",
                        value.class().name()
                    )));
                }
            }
        }
        Ok(zelf)
    }

    #[pymethod(magic)]
    fn int(zelf: PyRef<Self>) -> PyRef<Self> {
        zelf
    }

    #[pymethod(magic)]
    fn pos(&self) -> BigInt {
        self.value.clone()
    }

    #[pymethod(magic)]
    fn float(&self, vm: &VirtualMachine) -> PyResult<f64> {
        try_to_float(&self.value, vm)
    }

    #[pymethod(magic)]
    fn trunc(zelf: PyRef<Self>) -> PyRef<Self> {
        zelf
    }

    #[pymethod(magic)]
    fn floor(zelf: PyRef<Self>) -> PyRef<Self> {
        zelf
    }

    #[pymethod(magic)]
    fn ceil(zelf: PyRef<Self>) -> PyRef<Self> {
        zelf
    }

    #[pymethod(magic)]
    fn index(zelf: PyRef<Self>) -> PyRef<Self> {
        zelf
    }

    #[pymethod(magic)]
    fn invert(&self) -> BigInt {
        !(&self.value)
    }

    #[pymethod(magic)]
    pub(crate) fn repr(&self) -> String {
        self.value.to_string()
    }

    #[pymethod(magic)]
    fn format(&self, spec: PyStrRef, vm: &VirtualMachine) -> PyResult<String> {
        match FormatSpec::parse(spec.as_str())
            .and_then(|format_spec| format_spec.format_int(&self.value))
        {
            Ok(string) => Ok(string),
            Err(err) => Err(vm.new_value_error(err.to_string())),
        }
    }

    #[pymethod(magic)]
    fn bool(&self) -> bool {
        !self.value.is_zero()
    }

    #[pymethod(magic)]
    fn sizeof(&self) -> usize {
        size_of::<Self>() + (((self.value.bits() + 7) & !7) / 8) as usize
    }

    #[pymethod]
    fn as_integer_ratio(&self, vm: &VirtualMachine) -> (PyRef<Self>, i32) {
        (vm.ctx.new_bigint(&self.value), 1)
    }

    #[pymethod]
    fn bit_length(&self) -> u64 {
        self.value.bits()
    }

    #[pymethod]
    fn conjugate(zelf: PyRef<Self>) -> PyRef<Self> {
        zelf
    }

    #[pyclassmethod]
    fn from_bytes(
        cls: PyTypeRef,
        args: IntFromByteArgs,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<Self>> {
        let signed = if let OptionalArg::Present(signed) = args.signed {
            signed.to_bool()
        } else {
            false
        };

        let value = match (args.byteorder.as_str(), signed) {
            ("big", true) => BigInt::from_signed_bytes_be(&args.bytes.elements),
            ("big", false) => BigInt::from_bytes_be(Sign::Plus, &args.bytes.elements),
            ("little", true) => BigInt::from_signed_bytes_le(&args.bytes.elements),
            ("little", false) => BigInt::from_bytes_le(Sign::Plus, &args.bytes.elements),
            _ => {
                return Err(
                    vm.new_value_error("byteorder must be either 'little' or 'big'".to_owned())
                )
            }
        };
        Self::with_value(cls, value, vm)
    }

    #[pymethod]
    fn to_bytes(&self, args: IntToByteArgs, vm: &VirtualMachine) -> PyResult<PyBytes> {
        let signed = if let OptionalArg::Present(signed) = args.signed {
            signed.to_bool()
        } else {
            false
        };

        let value = self.as_bigint();
        if value.sign() == Sign::Minus && !signed {
            return Err(vm.new_overflow_error("can't convert negative int to unsigned".to_owned()));
        }

        let byte_len = args.length.try_to_primitive(vm)?;

        let mut origin_bytes = match (args.byteorder.as_str(), signed) {
            ("big", true) => value.to_signed_bytes_be(),
            ("big", false) => value.to_bytes_be().1,
            ("little", true) => value.to_signed_bytes_le(),
            ("little", false) => value.to_bytes_le().1,
            _ => {
                return Err(
                    vm.new_value_error("byteorder must be either 'little' or 'big'".to_owned())
                );
            }
        };

        let origin_len = origin_bytes.len();
        if origin_len > byte_len {
            return Err(vm.new_overflow_error("int too big to convert".to_owned()));
        }

        let mut append_bytes = match value.sign() {
            Sign::Minus => vec![255u8; byte_len - origin_len],
            _ => vec![0u8; byte_len - origin_len],
        };

        let bytes = match args.byteorder.as_str() {
            "big" => {
                let mut bytes = append_bytes;
                bytes.append(&mut origin_bytes);
                bytes
            }
            "little" => {
                let mut bytes = origin_bytes;
                bytes.append(&mut append_bytes);
                bytes
            }
            _ => Vec::new(),
        };
        Ok(bytes.into())
    }
    #[pyproperty]
    fn real(&self, vm: &VirtualMachine) -> PyRef<Self> {
        // subclasses must return int here
        vm.ctx.new_bigint(&self.value)
    }

    #[pyproperty]
    fn imag(&self) -> usize {
        0
    }

    #[pyproperty]
    fn numerator(zelf: PyRef<Self>) -> PyRef<Self> {
        zelf
    }

    #[pyproperty]
    fn denominator(&self) -> usize {
        1
    }

    #[pymethod]
    /// Returns the number of ones 1 an int. When the number is < 0,
    /// then it returns the number of ones of the absolute value.
    fn bit_count(&self) -> u32 {
        self.value.iter_u32_digits().map(|n| n.count_ones()).sum()
    }

    #[pymethod(magic)]
    fn getnewargs(&self, vm: &VirtualMachine) -> PyObjectRef {
        (self.value.clone(),).into_pyobject(vm)
    }
}

impl Comparable for PyInt {
    fn cmp(
        zelf: &crate::PyObjectView<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        let r = other
            .payload_if_subclass::<PyInt>(vm)
            .map(|other| op.eval_ord(zelf.value.cmp(&other.value)));
        Ok(PyComparisonValue::from_option(r))
    }
}

impl Hashable for PyInt {
    #[inline]
    fn hash(zelf: &crate::PyObjectView<Self>, _vm: &VirtualMachine) -> PyResult<hash::PyHash> {
        Ok(hash::hash_bigint(zelf.as_bigint()))
    }
}

#[derive(FromArgs)]
pub struct IntOptions {
    #[pyarg(positional, optional)]
    val_options: OptionalArg<PyObjectRef>,
    #[pyarg(any, optional)]
    base: OptionalArg<PyObjectRef>,
}

#[derive(FromArgs)]
struct IntFromByteArgs {
    bytes: PyBytesInner,
    byteorder: PyStrRef,
    #[pyarg(named, optional)]
    signed: OptionalArg<ArgIntoBool>,
}

#[derive(FromArgs)]
struct IntToByteArgs {
    length: PyIntRef,
    byteorder: PyStrRef,
    #[pyarg(named, optional)]
    signed: OptionalArg<ArgIntoBool>,
}

fn try_int_radix(obj: &PyObject, base: u32, vm: &VirtualMachine) -> PyResult<BigInt> {
    debug_assert!(base == 0 || (2..=36).contains(&base));

    let opt = match_class!(match obj.to_owned() {
        string @ PyStr => {
            let s = string.as_str();
            bytes_to_int(s.as_bytes(), base)
        }
        bytes @ PyBytes => {
            let bytes = bytes.as_bytes();
            bytes_to_int(bytes, base)
        }
        bytearray @ PyByteArray => {
            let inner = bytearray.borrow_buf();
            bytes_to_int(&inner, base)
        }
        _ => {
            return Err(
                vm.new_type_error("int() can't convert non-string with explicit base".to_owned())
            );
        }
    });
    match opt {
        Some(int) => Ok(int),
        None => Err(vm.new_value_error(format!(
            "invalid literal for int() with base {}: {}",
            base,
            obj.repr(vm)?,
        ))),
    }
}

fn bytes_to_int(lit: &[u8], mut base: u32) -> Option<BigInt> {
    // split sign
    let mut lit = lit.trim();
    let sign = match lit.first()? {
        b'+' => Some(Sign::Plus),
        b'-' => Some(Sign::Minus),
        _ => None,
    };
    if sign.is_some() {
        lit = &lit[1..];
    }

    // split radix
    let first = *lit.first()?;
    let has_radix = if first == b'0' {
        match base {
            0 => {
                if let Some(parsed) = lit.get(1).and_then(detect_base) {
                    base = parsed;
                    true
                } else {
                    if let [_first, ref others @ .., last] = lit {
                        let is_zero =
                            others.iter().all(|&c| c == b'0' || c == b'_') && *last == b'0';
                        if !is_zero {
                            return None;
                        }
                    }
                    return Some(BigInt::zero());
                }
            }
            16 => lit.get(1).map_or(false, |&b| matches!(b, b'x' | b'X')),
            2 => lit.get(1).map_or(false, |&b| matches!(b, b'b' | b'B')),
            8 => lit.get(1).map_or(false, |&b| matches!(b, b'o' | b'O')),
            _ => false,
        }
    } else {
        if base == 0 {
            base = 10;
        }
        false
    };
    if has_radix {
        lit = &lit[2..];
        if lit.first()? == &b'_' {
            lit = &lit[1..];
        }
    }

    // remove zeroes
    let mut last = *lit.first()?;
    if last == b'0' {
        let mut count = 0;
        for &cur in &lit[1..] {
            if cur == b'_' {
                if last == b'_' {
                    return None;
                }
            } else if cur != b'0' {
                break;
            };
            count += 1;
            last = cur;
        }
        let prefix_last = lit[count];
        lit = &lit[count + 1..];
        if lit.is_empty() && prefix_last == b'_' {
            return None;
        }
    }

    // validate
    for c in lit.iter() {
        let c = *c;
        if !(c.is_ascii_alphanumeric() || c == b'_') {
            return None;
        }

        if c == b'_' && last == b'_' {
            return None;
        }

        last = c;
    }
    if last == b'_' {
        return None;
    }

    // parse
    Some(if lit.is_empty() {
        BigInt::zero()
    } else {
        let uint = BigUint::parse_bytes(lit, base)?;
        BigInt::from_biguint(sign.unwrap_or(Sign::Plus), uint)
    })
}

fn detect_base(c: &u8) -> Option<u32> {
    match c {
        b'x' | b'X' => Some(16),
        b'b' | b'B' => Some(2),
        b'o' | b'O' => Some(8),
        _ => None,
    }
}

// Retrieve inner int value:
pub(crate) fn get_value(obj: &PyObject) -> &BigInt {
    &obj.payload::<PyInt>().unwrap().value
}

pub fn try_to_float(int: &BigInt, vm: &VirtualMachine) -> PyResult<f64> {
    i2f(int).ok_or_else(|| vm.new_overflow_error("int too large to convert to float".to_owned()))
}
// num-bigint now returns Some(inf) for to_f64() in some cases, so just keep that the same for now
fn i2f(int: &BigInt) -> Option<f64> {
    int.to_f64().filter(|f| f.is_finite())
}

pub(crate) fn try_int(obj: &PyObject, vm: &VirtualMachine) -> PyResult<BigInt> {
    fn try_convert(obj: &PyObject, lit: &[u8], vm: &VirtualMachine) -> PyResult<BigInt> {
        let base = 10;
        match bytes_to_int(lit, base) {
            Some(i) => Ok(i),
            None => Err(vm.new_value_error(format!(
                "invalid literal for int() with base {}: {}",
                base,
                obj.repr(vm)?,
            ))),
        }
    }

    // test for strings and bytes
    if let Some(s) = obj.downcast_ref::<PyStr>() {
        return try_convert(obj, s.as_str().as_bytes(), vm);
    }
    if let Ok(r) = obj.try_bytes_like(vm, |x| try_convert(obj, x, vm)) {
        return r;
    }
    // strict `int` check
    if let Some(int) = obj.payload_if_exact::<PyInt>(vm) {
        return Ok(int.as_bigint().clone());
    }
    // call __int__, then __index__, then __trunc__ (converting the __trunc__ result via  __index__ if needed)
    // TODO: using __int__ is deprecated and removed in Python 3.10
    if let Some(method) = vm.get_method(obj.to_owned(), "__int__") {
        let result = vm.invoke(&method?, ())?;
        return match result.payload::<PyInt>() {
            Some(int_obj) => Ok(int_obj.as_bigint().clone()),
            None => Err(vm.new_type_error(format!(
                "__int__ returned non-int (type '{}')",
                result.class().name()
            ))),
        };
    }
    // TODO: returning strict subclasses of int in __index__ is deprecated
    if let Some(r) = vm.to_index_opt(obj.to_owned()).transpose()? {
        return Ok(r.as_bigint().clone());
    }
    if let Some(method) = vm.get_method(obj.to_owned(), "__trunc__") {
        let result = vm.invoke(&method?, ())?;
        return vm
            .to_index_opt(result.clone())
            .unwrap_or_else(|| {
                Err(vm.new_type_error(format!(
                    "__trunc__ returned non-Integral (type '{}')",
                    result.class().name()
                )))
            })
            .map(|int_obj| int_obj.as_bigint().clone());
    }

    Err(vm.new_type_error(format!(
        "int() argument must be a string, a bytes-like object or a number, not '{}'",
        obj.class().name()
    )))
}

pub(crate) fn init(context: &PyContext) {
    PyInt::extend_class(context, &context.types.int_type);
}

#[test]
fn test_bytes_to_int() {
    assert_eq!(bytes_to_int(&b"0b101"[..], 2).unwrap(), BigInt::from(5));
    assert_eq!(bytes_to_int(&b"0x_10"[..], 16).unwrap(), BigInt::from(16));
    assert_eq!(bytes_to_int(&b"0b"[..], 16).unwrap(), BigInt::from(11));
    assert_eq!(bytes_to_int(&b"+0b101"[..], 2).unwrap(), BigInt::from(5));
    assert_eq!(bytes_to_int(&b"0_0_0"[..], 10).unwrap(), BigInt::from(0));
    assert_eq!(bytes_to_int(&b"09_99"[..], 0), None);
    assert_eq!(bytes_to_int(&b"000"[..], 0).unwrap(), BigInt::from(0));
    assert_eq!(bytes_to_int(&b"0_"[..], 0), None);
    assert_eq!(bytes_to_int(&b"0_100"[..], 10).unwrap(), BigInt::from(100));
}
