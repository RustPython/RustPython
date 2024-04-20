use super::{float, PyByteArray, PyBytes, PyStr, PyType, PyTypeRef};
use crate::{
    builtins::PyStrRef,
    bytesinner::PyBytesInner,
    class::PyClassImpl,
    common::{
        hash,
        int::{bigint_to_finite_float, bytes_to_int, true_div},
    },
    convert::{IntoPyException, ToPyObject, ToPyResult},
    function::{
        ArgByteOrder, ArgIntoBool, OptionalArg, OptionalOption, PyArithmeticValue,
        PyComparisonValue,
    },
    protocol::PyNumberMethods,
    types::{AsNumber, Comparable, Constructor, Hashable, PyComparisonOp, Representable},
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyRefExact, PyResult,
    TryFromBorrowedObject, VirtualMachine,
};
use malachite_bigint::{BigInt, Sign};
use num_integer::Integer;
use num_traits::{One, Pow, PrimInt, Signed, ToPrimitive, Zero};
use rustpython_format::FormatSpec;
use std::fmt;
use std::ops::{Neg, Not};

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

impl PyPayload for PyInt {
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.int_type
    }

    fn into_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_int(self.value).into()
    }
}

macro_rules! impl_into_pyobject_int {
    ($($t:ty)*) => {$(
        impl ToPyObject for $t {
            fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
                vm.ctx.new_int(self).into()
            }
        }
    )*};
}

impl_into_pyobject_int!(isize i8 i16 i32 i64 i128 usize u8 u16 u32 u64 u128 BigInt);

macro_rules! impl_try_from_object_int {
    ($(($t:ty, $to_prim:ident),)*) => {$(
        impl<'a> TryFromBorrowedObject<'a> for $t {
            fn try_from_borrowed_object(vm: &VirtualMachine, obj: &'a PyObject) -> PyResult<Self> {
                obj.try_value_with(|int: &PyInt| {
                    int.try_to_primitive(vm)
                }, vm)
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

fn inner_lshift(base: &BigInt, bits: &BigInt, vm: &VirtualMachine) -> PyResult {
    inner_shift(
        base,
        bits,
        |base, bits| base << bits,
        |bits, vm| {
            bits.to_usize().ok_or_else(|| {
                vm.new_overflow_error("the number is too large to convert to int".to_owned())
            })
        },
        vm,
    )
}

fn inner_rshift(base: &BigInt, bits: &BigInt, vm: &VirtualMachine) -> PyResult {
    inner_shift(
        base,
        bits,
        |base, bits| base >> bits,
        |bits, _vm| Ok(bits.to_usize().unwrap_or(usize::MAX)),
        vm,
    )
}

fn inner_shift<F, S>(
    base: &BigInt,
    bits: &BigInt,
    shift_op: F,
    shift_bits: S,
    vm: &VirtualMachine,
) -> PyResult
where
    F: Fn(&BigInt, usize) -> BigInt,
    S: Fn(&BigInt, &VirtualMachine) -> PyResult<usize>,
{
    if bits.is_negative() {
        Err(vm.new_value_error("negative shift count".to_owned()))
    } else if base.is_zero() {
        Ok(vm.ctx.new_int(0).into())
    } else {
        shift_bits(bits, vm).map(|bits| vm.ctx.new_int(shift_op(base, bits)).into())
    }
}

fn inner_truediv(i1: &BigInt, i2: &BigInt, vm: &VirtualMachine) -> PyResult {
    if i2.is_zero() {
        return Err(vm.new_zero_division_error("division by zero".to_owned()));
    }

    let float = true_div(i1, i2);

    if float.is_infinite() {
        Err(vm.new_exception_msg(
            vm.ctx.exceptions.overflow_error.to_owned(),
            "integer division result too large for a float".to_owned(),
        ))
    } else {
        Ok(vm.ctx.new_float(float).into())
    }
}

impl Constructor for PyInt {
    type Args = IntOptions;

    fn py_new(cls: PyTypeRef, options: Self::Args, vm: &VirtualMachine) -> PyResult {
        if cls.is(vm.ctx.types.bool_type) {
            return Err(
                vm.new_type_error("int.__new__(bool) is not safe, use bool.__new__()".to_owned())
            );
        }

        let value = if let OptionalArg::Present(val) = options.val_options {
            if let OptionalArg::Present(base) = options.base {
                let base = base
                    .try_index(vm)?
                    .as_bigint()
                    .to_u32()
                    .filter(|&v| v == 0 || (2..=36).contains(&v))
                    .ok_or_else(|| {
                        vm.new_value_error("int() base must be >= 2 and <= 36, or 0".to_owned())
                    })?;
                try_int_radix(&val, base, vm)
            } else {
                let val = if cls.is(vm.ctx.types.int_type) {
                    match val.downcast_exact::<PyInt>(vm) {
                        Ok(i) => {
                            return Ok(i.into_pyref().into());
                        }
                        Err(val) => val,
                    }
                } else {
                    val
                };

                val.try_int(vm).map(|x| x.as_bigint().clone())
            }
        } else if let OptionalArg::Present(_) = options.base {
            Err(vm.new_type_error("int() missing string argument".to_owned()))
        } else {
            Ok(Zero::zero())
        }?;

        Self::with_value(cls, value, vm).to_pyresult(vm)
    }
}

impl PyInt {
    fn with_value<T>(cls: PyTypeRef, value: T, vm: &VirtualMachine) -> PyResult<PyRef<Self>>
    where
        T: Into<BigInt> + ToPrimitive,
    {
        if cls.is(vm.ctx.types.int_type) {
            Ok(vm.ctx.new_int(value))
        } else if cls.is(vm.ctx.types.bool_type) {
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
                    Sign::Minus => out * -1i32 as u32,
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

#[pyclass(
    flags(BASETYPE),
    with(PyRef, Comparable, Hashable, Constructor, AsNumber, Representable)
)]
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
        self.general_op(other, |a, b| inner_lshift(a, b, vm), vm)
    }

    #[pymethod(magic)]
    fn rlshift(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.general_op(other, |a, b| inner_lshift(b, a, vm), vm)
    }

    #[pymethod(magic)]
    fn rshift(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.general_op(other, |a, b| inner_rshift(a, b, vm), vm)
    }

    #[pymethod(magic)]
    fn rrshift(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.general_op(other, |a, b| inner_rshift(b, a, vm), vm)
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

    fn modpow(&self, other: PyObjectRef, modulus: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let modulus = match modulus.payload_if_subclass::<PyInt>(vm) {
            Some(val) => val.as_bigint(),
            None => return Ok(vm.ctx.not_implemented()),
        };
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

    #[pymethod(magic)]
    fn pow(
        &self,
        other: PyObjectRef,
        r#mod: OptionalOption<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        match r#mod.flatten() {
            Some(modulus) => self.modpow(other, modulus, vm),
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
        ndigits: OptionalArg<PyIntRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<Self>> {
        if let OptionalArg::Present(ndigits) = ndigits {
            let ndigits = ndigits.as_bigint();
            // round(12345, -2) == 12300
            // If precision >= 0, then any integer is already rounded correctly
            if let Some(ndigits) = ndigits.neg().to_u32() {
                if ndigits > 0 {
                    // Work with positive integers and negate at the end if necessary
                    let sign = if zelf.value.is_negative() {
                        BigInt::from(-1)
                    } else {
                        BigInt::from(1)
                    };
                    let value = zelf.value.abs();

                    // Divide and multiply by the power of 10 to get the approximate answer
                    let pow10 = BigInt::from(10).pow(ndigits);
                    let quotient = &value / &pow10;
                    let rounded = &quotient * &pow10;

                    // Malachite division uses floor rounding, Python uses half-even
                    let remainder = &value - &rounded;
                    let halfpow10 = &pow10 / BigInt::from(2);
                    let correction =
                        if remainder > halfpow10 || (remainder == halfpow10 && quotient.is_odd()) {
                            pow10
                        } else {
                            BigInt::from(0)
                        };
                    let rounded = (rounded + correction) * sign;
                    return Ok(vm.ctx.new_int(rounded));
                }
            }
        }
        Ok(zelf)
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
    fn trunc(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyRefExact<Self> {
        zelf.int(vm)
    }

    #[pymethod(magic)]
    fn floor(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyRefExact<Self> {
        zelf.int(vm)
    }

    #[pymethod(magic)]
    fn ceil(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyRefExact<Self> {
        zelf.int(vm)
    }

    #[pymethod(magic)]
    fn index(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyRefExact<Self> {
        zelf.int(vm)
    }

    #[pymethod(magic)]
    fn invert(&self) -> BigInt {
        !(&self.value)
    }

    #[pymethod(magic)]
    fn format(&self, spec: PyStrRef, vm: &VirtualMachine) -> PyResult<String> {
        FormatSpec::parse(spec.as_str())
            .and_then(|format_spec| format_spec.format_int(&self.value))
            .map_err(|err| err.into_pyexception(vm))
    }

    #[pymethod(magic)]
    fn bool(&self) -> bool {
        !self.value.is_zero()
    }

    #[pymethod(magic)]
    fn sizeof(&self) -> usize {
        std::mem::size_of::<Self>() + (((self.value.bits() + 7) & !7) / 8) as usize
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
    fn conjugate(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyRefExact<Self> {
        zelf.int(vm)
    }

    #[pyclassmethod]
    fn from_bytes(
        cls: PyTypeRef,
        args: IntFromByteArgs,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<Self>> {
        let signed = args.signed.map_or(false, Into::into);
        let value = match (args.byteorder, signed) {
            (ArgByteOrder::Big, true) => BigInt::from_signed_bytes_be(args.bytes.as_bytes()),
            (ArgByteOrder::Big, false) => BigInt::from_bytes_be(Sign::Plus, args.bytes.as_bytes()),
            (ArgByteOrder::Little, true) => BigInt::from_signed_bytes_le(args.bytes.as_bytes()),
            (ArgByteOrder::Little, false) => {
                BigInt::from_bytes_le(Sign::Plus, args.bytes.as_bytes())
            }
        };
        Self::with_value(cls, value, vm)
    }

    #[pymethod]
    fn to_bytes(&self, args: IntToByteArgs, vm: &VirtualMachine) -> PyResult<PyBytes> {
        let signed = args.signed.map_or(false, Into::into);
        let byte_len = args.length;

        let value = self.as_bigint();
        match value.sign() {
            Sign::Minus if !signed => {
                return Err(
                    vm.new_overflow_error("can't convert negative int to unsigned".to_owned())
                )
            }
            Sign::NoSign => return Ok(vec![0u8; byte_len].into()),
            _ => {}
        }

        let mut origin_bytes = match (args.byteorder, signed) {
            (ArgByteOrder::Big, true) => value.to_signed_bytes_be(),
            (ArgByteOrder::Big, false) => value.to_bytes_be().1,
            (ArgByteOrder::Little, true) => value.to_signed_bytes_le(),
            (ArgByteOrder::Little, false) => value.to_bytes_le().1,
        };

        let origin_len = origin_bytes.len();
        if origin_len > byte_len {
            return Err(vm.new_overflow_error("int too big to convert".to_owned()));
        }

        let mut append_bytes = match value.sign() {
            Sign::Minus => vec![255u8; byte_len - origin_len],
            _ => vec![0u8; byte_len - origin_len],
        };

        let bytes = match args.byteorder {
            ArgByteOrder::Big => {
                let mut bytes = append_bytes;
                bytes.append(&mut origin_bytes);
                bytes
            }
            ArgByteOrder::Little => {
                let mut bytes = origin_bytes;
                bytes.append(&mut append_bytes);
                bytes
            }
        };
        Ok(bytes.into())
    }

    #[pygetset]
    fn real(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyRefExact<Self> {
        zelf.int(vm)
    }

    #[pygetset]
    fn imag(&self) -> usize {
        0
    }

    #[pygetset]
    fn numerator(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyRefExact<Self> {
        zelf.int(vm)
    }

    #[pygetset]
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
        (self.value.clone(),).to_pyobject(vm)
    }
}

#[pyclass]
impl PyRef<PyInt> {
    #[pymethod(magic)]
    fn int(self, vm: &VirtualMachine) -> PyRefExact<PyInt> {
        self.into_exact_or(&vm.ctx, |zelf| unsafe {
            // TODO: this is actually safe. we need better interface
            PyRefExact::new_unchecked(vm.ctx.new_bigint(&zelf.value))
        })
    }
}

impl Comparable for PyInt {
    fn cmp(
        zelf: &Py<Self>,
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

impl Representable for PyInt {
    #[inline]
    fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
        Ok(zelf.value.to_string())
    }
}

impl Hashable for PyInt {
    #[inline]
    fn hash(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<hash::PyHash> {
        Ok(hash::hash_bigint(zelf.as_bigint()))
    }
}

impl AsNumber for PyInt {
    fn as_number() -> &'static PyNumberMethods {
        static AS_NUMBER: PyNumberMethods = PyInt::AS_NUMBER;
        &AS_NUMBER
    }

    #[inline]
    fn clone_exact(zelf: &Py<Self>, vm: &VirtualMachine) -> PyRef<Self> {
        vm.ctx.new_bigint(&zelf.value)
    }
}

impl PyInt {
    pub(super) const AS_NUMBER: PyNumberMethods = PyNumberMethods {
        add: Some(|a, b, vm| PyInt::number_op(a, b, |a, b, _vm| a + b, vm)),
        subtract: Some(|a, b, vm| PyInt::number_op(a, b, |a, b, _vm| a - b, vm)),
        multiply: Some(|a, b, vm| PyInt::number_op(a, b, |a, b, _vm| a * b, vm)),
        remainder: Some(|a, b, vm| PyInt::number_op(a, b, inner_mod, vm)),
        divmod: Some(|a, b, vm| PyInt::number_op(a, b, inner_divmod, vm)),
        power: Some(|a, b, c, vm| {
            if let (Some(a), Some(b)) = (
                a.payload::<Self>(),
                if b.payload_is::<Self>() {
                    Some(b)
                } else {
                    None
                },
            ) {
                if vm.is_none(c) {
                    a.general_op(b.to_owned(), |a, b| inner_pow(a, b, vm), vm)
                } else {
                    a.modpow(b.to_owned(), c.to_owned(), vm)
                }
            } else {
                Ok(vm.ctx.not_implemented())
            }
        }),
        negative: Some(|num, vm| (&PyInt::number_downcast(num).value).neg().to_pyresult(vm)),
        positive: Some(|num, vm| Ok(PyInt::number_downcast_exact(num, vm).into())),
        absolute: Some(|num, vm| PyInt::number_downcast(num).value.abs().to_pyresult(vm)),
        boolean: Some(|num, _vm| Ok(PyInt::number_downcast(num).value.is_zero())),
        invert: Some(|num, vm| (&PyInt::number_downcast(num).value).not().to_pyresult(vm)),
        lshift: Some(|a, b, vm| PyInt::number_op(a, b, inner_lshift, vm)),
        rshift: Some(|a, b, vm| PyInt::number_op(a, b, inner_rshift, vm)),
        and: Some(|a, b, vm| PyInt::number_op(a, b, |a, b, _vm| a & b, vm)),
        xor: Some(|a, b, vm| PyInt::number_op(a, b, |a, b, _vm| a ^ b, vm)),
        or: Some(|a, b, vm| PyInt::number_op(a, b, |a, b, _vm| a | b, vm)),
        int: Some(|num, vm| Ok(PyInt::number_downcast_exact(num, vm).into())),
        float: Some(|num, vm| {
            let zelf = PyInt::number_downcast(num);
            try_to_float(&zelf.value, vm).map(|x| vm.ctx.new_float(x).into())
        }),
        floor_divide: Some(|a, b, vm| PyInt::number_op(a, b, inner_floordiv, vm)),
        true_divide: Some(|a, b, vm| PyInt::number_op(a, b, inner_truediv, vm)),
        index: Some(|num, vm| Ok(PyInt::number_downcast_exact(num, vm).into())),
        ..PyNumberMethods::NOT_IMPLEMENTED
    };

    fn number_op<F, R>(a: &PyObject, b: &PyObject, op: F, vm: &VirtualMachine) -> PyResult
    where
        F: FnOnce(&BigInt, &BigInt, &VirtualMachine) -> R,
        R: ToPyResult,
    {
        if let (Some(a), Some(b)) = (a.payload::<Self>(), b.payload::<Self>()) {
            op(&a.value, &b.value, vm).to_pyresult(vm)
        } else {
            Ok(vm.ctx.not_implemented())
        }
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
    #[pyarg(any, default = "ArgByteOrder::Big")]
    byteorder: ArgByteOrder,
    #[pyarg(named, optional)]
    signed: OptionalArg<ArgIntoBool>,
}

#[derive(FromArgs)]
struct IntToByteArgs {
    #[pyarg(any, default = "1")]
    length: usize,
    #[pyarg(any, default = "ArgByteOrder::Big")]
    byteorder: ArgByteOrder,
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

// Retrieve inner int value:
pub(crate) fn get_value(obj: &PyObject) -> &BigInt {
    &obj.payload::<PyInt>().unwrap().value
}

pub fn try_to_float(int: &BigInt, vm: &VirtualMachine) -> PyResult<f64> {
    bigint_to_finite_float(int)
        .ok_or_else(|| vm.new_overflow_error("int too large to convert to float".to_owned()))
}

pub(crate) fn init(context: &Context) {
    PyInt::extend_class(context, context.types.int_type);
}
