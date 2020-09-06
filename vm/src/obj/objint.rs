use std::fmt;
use std::mem::size_of;

use bstr::ByteSlice;
use num_bigint::{BigInt, BigUint, Sign};
use num_integer::Integer;
use num_traits::{One, Pow, Signed, ToPrimitive, Zero};

use super::objbool::IntoPyBool;
use super::objbytearray::PyByteArray;
use super::objbytes::PyBytes;
use super::objfloat;
use super::objmemory::PyMemoryView;
use super::objstr::{PyString, PyStringRef};
use super::objtype::PyClassRef;
use crate::bytesinner::PyBytesInner;
use crate::format::FormatSpec;
use crate::function::{OptionalArg, PyFuncArgs};
use crate::pyobject::{
    BorrowValue, IdProtocol, IntoPyObject, IntoPyResult, PyArithmaticValue, PyClassImpl,
    PyComparisonValue, PyContext, PyObjectRef, PyRef, PyResult, PyValue, TryFromObject,
    TypeProtocol,
};
use crate::stdlib::array::PyArray;
use crate::vm::VirtualMachine;
use rustpython_common::hash;

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

impl<'a> BorrowValue<'a> for PyInt {
    type Borrowed = &'a BigInt;

    fn borrow_value(&'a self) -> Self::Borrowed {
        &self.value
    }
}

impl<T> From<T> for PyInt
where
    T: Into<BigInt>,
{
    fn from(v: T) -> Self {
        Self { value: v.into() }
    }
}

impl PyValue for PyInt {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.types.int_type.clone()
    }

    fn into_object(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_int(self.value)
    }
}

macro_rules! impl_into_pyobject_int {
    ($($t:ty)*) => {$(
        impl IntoPyObject for $t {
            fn into_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
                vm.ctx.new_int(self)
            }
        }
    )*};
}

impl_into_pyobject_int!(isize i8 i16 i32 i64 usize u8 u16 u32 u64 BigInt);

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
                        ).to_owned())
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

fn inner_pow(int1: &BigInt, int2: &BigInt, vm: &VirtualMachine) -> PyResult {
    if int2.is_negative() {
        let v1 = try_float(int1, vm)?;
        let v2 = try_float(int2, vm)?;
        objfloat::float_pow(v1, v2, vm).into_pyresult(vm)
    } else {
        Ok(if let Some(v2) = int2.to_u64() {
            vm.ctx.new_int(Pow::pow(int1, v2))
        } else if int1.is_one() {
            vm.ctx.new_int(1)
        } else if int1.is_zero() {
            vm.ctx.new_int(0)
        } else if int1 == &BigInt::from(-1) {
            if int2.is_odd() {
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

fn inner_mod(int1: &BigInt, int2: &BigInt, vm: &VirtualMachine) -> PyResult {
    if int2.is_zero() {
        Err(vm.new_zero_division_error("integer modulo by zero".to_owned()))
    } else {
        Ok(vm.ctx.new_int(int1.mod_floor(int2)))
    }
}

fn inner_floordiv(int1: &BigInt, int2: &BigInt, vm: &VirtualMachine) -> PyResult {
    if int2.is_zero() {
        Err(vm.new_zero_division_error("integer division by zero".to_owned()))
    } else {
        Ok(vm.ctx.new_int(int1.div_floor(&int2)))
    }
}

fn inner_divmod(int1: &BigInt, int2: &BigInt, vm: &VirtualMachine) -> PyResult {
    if int2.is_zero() {
        Err(vm.new_zero_division_error("integer division or modulo by zero".to_owned()))
    } else {
        let (div, modulo) = int1.div_mod_floor(int2);
        Ok(vm
            .ctx
            .new_tuple(vec![vm.ctx.new_int(div), vm.ctx.new_int(modulo)]))
    }
}

fn inner_shift<F>(int1: &BigInt, int2: &BigInt, shift_op: F, vm: &VirtualMachine) -> PyResult
where
    F: Fn(&BigInt, usize) -> BigInt,
{
    if int2.is_negative() {
        Err(vm.new_value_error("negative shift count".to_owned()))
    } else if int1.is_zero() {
        Ok(vm.ctx.new_int(0))
    } else {
        let int2 = int2.to_usize().ok_or_else(|| {
            vm.new_overflow_error("the number is too large to convert to int".to_owned())
        })?;
        Ok(vm.ctx.new_int(shift_op(int1, int2)))
    }
}

#[inline]
fn inner_truediv(i1: &BigInt, i2: &BigInt, vm: &VirtualMachine) -> PyResult {
    if i2.is_zero() {
        return Err(vm.new_zero_division_error("integer division by zero".to_owned()));
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
            Err(vm.new_overflow_error("int too large to convert to float".to_owned()))
        }
    }
}

#[pyimpl(flags(BASETYPE))]
impl PyInt {
    fn with_value<T>(cls: PyClassRef, value: T, vm: &VirtualMachine) -> PyResult<PyIntRef>
    where
        T: Into<BigInt> + ToPrimitive,
    {
        if cls.is(&vm.ctx.types.int_type) {
            Ok(vm.ctx.new_int(value).downcast().unwrap())
        } else if cls.is(&vm.ctx.types.bool_type) {
            Ok(vm
                .ctx
                .new_bool(!value.into().eq(&BigInt::zero()))
                .downcast()
                .unwrap())
        } else {
            PyInt::from(value).into_ref_with_type(vm, cls)
        }
    }

    #[pyslot]
    fn tp_new(cls: PyClassRef, options: IntOptions, vm: &VirtualMachine) -> PyResult<PyIntRef> {
        let value = if let OptionalArg::Present(val) = options.val_options {
            if let OptionalArg::Present(base) = options.base {
                let base = vm
                    .to_index(&base)
                    .unwrap_or_else(|| {
                        Err(vm.new_type_error(format!(
                            "'{}' object cannot be interpreted as an integer",
                            base.lease_class().name
                        )))
                    })?
                    .borrow_value()
                    .to_u32()
                    .filter(|&v| v == 0 || (2..=36).contains(&v))
                    .ok_or_else(|| {
                        vm.new_value_error("int() base must be >= 2 and <= 36, or 0".to_owned())
                    })?;
                to_int_radix(vm, &val, base)
            } else {
                to_int(vm, &val)
            }
        } else if let OptionalArg::Present(_) = options.base {
            Err(vm.new_type_error("int() missing string argument".to_owned()))
        } else {
            Ok(Zero::zero())
        }?;

        Self::with_value(cls, value, vm)
    }

    #[inline]
    fn cmp<F>(&self, other: PyObjectRef, op: F, vm: &VirtualMachine) -> PyComparisonValue
    where
        F: Fn(&BigInt, &BigInt) -> bool,
    {
        let r = other
            .payload_if_subclass::<PyInt>(vm)
            .map(|other| op(&self.value, &other.value));
        PyComparisonValue::from_option(r)
    }

    #[pymethod(name = "__eq__")]
    fn eq(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyComparisonValue {
        self.cmp(other, |a, b| a == b, vm)
    }

    #[pymethod(name = "__ne__")]
    fn ne(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyComparisonValue {
        self.cmp(other, |a, b| a != b, vm)
    }

    #[pymethod(name = "__lt__")]
    fn lt(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyComparisonValue {
        self.cmp(other, |a, b| a < b, vm)
    }

    #[pymethod(name = "__le__")]
    fn le(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyComparisonValue {
        self.cmp(other, |a, b| a <= b, vm)
    }

    #[pymethod(name = "__gt__")]
    fn gt(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyComparisonValue {
        self.cmp(other, |a, b| a > b, vm)
    }

    #[pymethod(name = "__ge__")]
    fn ge(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyComparisonValue {
        self.cmp(other, |a, b| a >= b, vm)
    }

    #[inline]
    fn int_op<F>(&self, other: PyObjectRef, op: F, vm: &VirtualMachine) -> PyArithmaticValue<BigInt>
    where
        F: Fn(&BigInt, &BigInt) -> BigInt,
    {
        let r = other
            .payload_if_subclass::<PyInt>(vm)
            .map(|other| op(&self.value, &other.value));
        PyArithmaticValue::from_option(r)
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

    #[pymethod(name = "__add__")]
    fn add(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyArithmaticValue<BigInt> {
        self.int_op(other, |a, b| a + b, vm)
    }

    #[pymethod(name = "__radd__")]
    fn radd(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyArithmaticValue<BigInt> {
        self.add(other, vm)
    }

    #[pymethod(name = "__sub__")]
    fn sub(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyArithmaticValue<BigInt> {
        self.int_op(other, |a, b| a - b, vm)
    }

    #[pymethod(name = "__rsub__")]
    fn rsub(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyArithmaticValue<BigInt> {
        self.int_op(other, |a, b| b - a, vm)
    }

    #[pymethod(name = "__mul__")]
    fn mul(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyArithmaticValue<BigInt> {
        self.int_op(other, |a, b| a * b, vm)
    }

    #[pymethod(name = "__rmul__")]
    fn rmul(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyArithmaticValue<BigInt> {
        self.mul(other, vm)
    }

    #[pymethod(name = "__truediv__")]
    fn truediv(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.general_op(other, |a, b| inner_truediv(a, b, vm), vm)
    }

    #[pymethod(name = "__rtruediv__")]
    fn rtruediv(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.general_op(other, |a, b| inner_truediv(b, a, vm), vm)
    }

    #[pymethod(name = "__floordiv__")]
    fn floordiv(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.general_op(other, |a, b| inner_floordiv(a, b, &vm), vm)
    }

    #[pymethod(name = "__rfloordiv__")]
    fn rfloordiv(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.general_op(other, |a, b| inner_floordiv(b, a, &vm), vm)
    }

    #[pymethod(name = "__lshift__")]
    fn lshift(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.general_op(other, |a, b| inner_shift(a, b, |a, b| a << b, vm), vm)
    }

    #[pymethod(name = "__rlshift__")]
    fn rlshift(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.general_op(other, |a, b| inner_shift(b, a, |a, b| a << b, vm), vm)
    }

    #[pymethod(name = "__rshift__")]
    fn rshift(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.general_op(other, |a, b| inner_shift(a, b, |a, b| a >> b, vm), vm)
    }

    #[pymethod(name = "__rrshift__")]
    fn rrshift(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.general_op(other, |a, b| inner_shift(b, a, |a, b| a >> b, vm), vm)
    }

    #[pymethod(name = "__xor__")]
    pub fn xor(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyArithmaticValue<BigInt> {
        self.int_op(other, |a, b| a ^ b, vm)
    }

    #[pymethod(name = "__rxor__")]
    fn rxor(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyArithmaticValue<BigInt> {
        self.xor(other, vm)
    }

    #[pymethod(name = "__or__")]
    pub fn or(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyArithmaticValue<BigInt> {
        self.int_op(other, |a, b| a | b, vm)
    }

    #[pymethod(name = "__ror__")]
    fn ror(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyArithmaticValue<BigInt> {
        self.or(other, vm)
    }

    #[pymethod(name = "__and__")]
    pub fn and(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyArithmaticValue<BigInt> {
        self.int_op(other, |a, b| a & b, vm)
    }

    #[pymethod(name = "__rand__")]
    fn rand(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyArithmaticValue<BigInt> {
        self.and(other, vm)
    }

    #[pymethod(name = "__pow__")]
    fn pow(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.general_op(other, |a, b| inner_pow(a, b, vm), vm)
    }

    #[pymethod(name = "__rpow__")]
    fn rpow(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.general_op(other, |a, b| inner_pow(b, a, vm), vm)
    }

    #[pymethod(name = "__mod__")]
    fn mod_(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.general_op(other, |a, b| inner_mod(a, b, vm), vm)
    }

    #[pymethod(name = "__rmod__")]
    fn rmod(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.general_op(other, |a, b| inner_mod(b, a, vm), vm)
    }

    #[pymethod(name = "__divmod__")]
    fn divmod(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.general_op(other, |a, b| inner_divmod(a, b, vm), vm)
    }

    #[pymethod(name = "__rdivmod__")]
    fn rdivmod(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.general_op(other, |a, b| inner_divmod(b, a, vm), vm)
    }

    #[pymethod(name = "__neg__")]
    fn neg(&self) -> BigInt {
        -(&self.value)
    }

    #[pymethod(name = "__hash__")]
    fn hash(&self) -> hash::PyHash {
        hash::hash_bigint(&self.value)
    }

    #[pymethod(name = "__abs__")]
    fn abs(&self) -> BigInt {
        self.value.abs()
    }

    #[pymethod(name = "__round__")]
    fn round(
        zelf: PyRef<Self>,
        precision: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyIntRef> {
        match precision {
            OptionalArg::Missing => (),
            OptionalArg::Present(ref value) => {
                if !vm.get_none().is(value) {
                    // Only accept int type ndigits
                    let _ndigits = value.payload_if_subclass::<PyInt>(vm).ok_or_else(|| {
                        vm.new_type_error(format!(
                            "'{}' object cannot be interpreted as an integer",
                            value.lease_class().name
                        ))
                    })?;
                } else {
                    return Err(vm.new_type_error(format!(
                        "'{}' object cannot be interpreted as an integer",
                        value.lease_class().name
                    )));
                }
            }
        }
        Ok(zelf)
    }

    #[pymethod(name = "__int__")]
    fn int(zelf: PyRef<Self>) -> PyIntRef {
        zelf
    }

    #[pymethod(name = "__pos__")]
    fn pos(&self) -> BigInt {
        self.value.clone()
    }

    #[pymethod(name = "__float__")]
    fn float(&self, vm: &VirtualMachine) -> PyResult<f64> {
        try_float(&self.value, vm)
    }

    #[pymethod(name = "__trunc__")]
    fn trunc(zelf: PyRef<Self>) -> PyIntRef {
        zelf
    }

    #[pymethod(name = "__floor__")]
    fn floor(zelf: PyRef<Self>) -> PyIntRef {
        zelf
    }

    #[pymethod(name = "__ceil__")]
    fn ceil(zelf: PyRef<Self>) -> PyIntRef {
        zelf
    }

    #[pymethod(name = "__index__")]
    fn index(zelf: PyRef<Self>) -> PyIntRef {
        zelf
    }

    #[pymethod(name = "__invert__")]
    fn invert(&self) -> BigInt {
        !(&self.value)
    }

    #[pymethod(name = "__repr__")]
    pub(crate) fn repr(&self) -> String {
        self.value.to_string()
    }

    #[pymethod(name = "__format__")]
    fn format(&self, spec: PyStringRef, vm: &VirtualMachine) -> PyResult<String> {
        match FormatSpec::parse(spec.borrow_value())
            .and_then(|format_spec| format_spec.format_int(&self.value))
        {
            Ok(string) => Ok(string),
            Err(err) => Err(vm.new_value_error(err.to_string())),
        }
    }

    #[pymethod(name = "__bool__")]
    fn bool(&self) -> bool {
        !self.value.is_zero()
    }

    #[pymethod(name = "__sizeof__")]
    fn sizeof(&self) -> usize {
        size_of::<Self>() + (((self.value.bits() + 7) & !7) / 8) as usize
    }

    #[pymethod(name = "as_integer_ratio")]
    fn as_integer_ratio(&self, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.new_tuple(vec![
            vm.ctx.new_bigint(&self.value),
            vm.ctx.new_bigint(&BigInt::one()),
        ]))
    }

    #[pymethod]
    fn bit_length(&self) -> u64 {
        self.value.bits()
    }

    #[pymethod]
    fn conjugate(zelf: PyRef<Self>) -> PyIntRef {
        zelf
    }

    #[pyclassmethod]
    fn from_bytes(
        cls: PyClassRef,
        args: IntFromByteArgs,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<Self>> {
        let signed = if let OptionalArg::Present(signed) = args.signed {
            signed.to_bool()
        } else {
            false
        };

        let value = match (args.byteorder.borrow_value(), signed) {
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

        let value = self.borrow_value();
        if value.sign() == Sign::Minus && !signed {
            return Err(vm.new_overflow_error("can't convert negative int to unsigned".to_owned()));
        }

        let byte_len = args.length.borrow_value().to_usize().ok_or_else(|| {
            vm.new_overflow_error("Python int too large to convert to C ssize_t".to_owned())
        })?;

        let mut origin_bytes = match (args.byteorder.borrow_value(), signed) {
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

        let bytes = match args.byteorder.borrow_value() {
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
    fn real(&self, vm: &VirtualMachine) -> PyObjectRef {
        // subclasses must return int here
        vm.ctx.new_bigint(&self.value)
    }

    #[pyproperty]
    fn imag(&self) -> usize {
        0
    }

    #[pyproperty]
    fn numerator(zelf: PyRef<Self>) -> PyIntRef {
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
        self.value
            .to_u32_digits()
            .1
            .iter()
            .map(|n| n.count_ones())
            .sum()
    }
}

#[derive(FromArgs)]
struct IntOptions {
    #[pyarg(positional_only, optional = true)]
    val_options: OptionalArg<PyObjectRef>,
    #[pyarg(positional_or_keyword, optional = true)]
    base: OptionalArg<PyObjectRef>,
}

#[derive(FromArgs)]
struct IntFromByteArgs {
    #[pyarg(positional_or_keyword)]
    bytes: PyBytesInner,
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
pub(crate) fn to_int(vm: &VirtualMachine, obj: &PyObjectRef) -> PyResult<BigInt> {
    let base = 10;
    let opt = match_class!(match obj.clone() {
        string @ PyString => {
            let s = string.borrow_value();
            bytes_to_int(s.as_bytes(), base)
        }
        bytes @ PyBytes => {
            let bytes = bytes.borrow_value();
            bytes_to_int(bytes, base)
        }
        bytearray @ PyByteArray => {
            let inner = bytearray.borrow_value();
            bytes_to_int(&inner.elements, base)
        }
        memoryview @ PyMemoryView => {
            // TODO: proper error handling instead of `unwrap()`
            memoryview
                .try_bytes(|bytes| bytes_to_int(&bytes, base))
                .unwrap()
        }
        array @ PyArray => {
            let bytes = array.tobytes();
            bytes_to_int(&bytes, base)
        }
        obj => {
            let method = vm.get_method_or_type_error(obj.clone(), "__int__", || {
                format!(
                    "int() argument must be a string or a number, not '{}'",
                    obj.class().name
                )
            })?;
            let result = vm.invoke(&method, PyFuncArgs::default())?;
            return match result.payload::<PyInt>() {
                Some(int_obj) => Ok(int_obj.borrow_value().clone()),
                None => Err(vm.new_type_error(format!(
                    "TypeError: __int__ returned non-int (type '{}')",
                    result.class().name
                ))),
            };
        }
    });
    match opt {
        Some(int) => Ok(int),
        None => Err(vm.new_value_error(format!(
            "invalid literal for int() with base {}: {}",
            base,
            vm.to_repr(obj)?,
        ))),
    }
}

fn to_int_radix(vm: &VirtualMachine, obj: &PyObjectRef, base: u32) -> PyResult<BigInt> {
    debug_assert!(base == 0 || (2..=36).contains(&base));

    let opt = match_class!(match obj.clone() {
        string @ PyString => {
            let s = string.borrow_value();
            bytes_to_int(s.as_bytes(), base)
        }
        bytes @ PyBytes => {
            let bytes = bytes.borrow_value();
            bytes_to_int(bytes, base)
        }
        bytearray @ PyByteArray => {
            let inner = bytearray.borrow_value();
            bytes_to_int(&inner.elements, base)
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
            vm.to_repr(obj)?,
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
        let uint = BigUint::parse_bytes(&lit, base)?;
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
pub fn get_value(obj: &PyObjectRef) -> &BigInt {
    &obj.payload::<PyInt>().unwrap().value
}

pub fn try_float(int: &BigInt, vm: &VirtualMachine) -> PyResult<f64> {
    int.to_f64()
        .ok_or_else(|| vm.new_overflow_error("int too large to convert to float".to_owned()))
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
