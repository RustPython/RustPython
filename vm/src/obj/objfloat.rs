use hexf_parse;
use num_bigint::{BigInt, ToBigInt};
use num_rational::Ratio;
use num_traits::{float::Float, pow, sign::Signed, ToPrimitive, Zero};

use super::objbytes;
use super::objint;
use super::objstr::{self, PyStringRef};
use super::objtype::{self, PyClassRef};
use crate::function::OptionalArg;
use crate::pyhash;
use crate::pyobject::{
    IdProtocol, IntoPyObject, PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue,
    TryFromObject, TypeProtocol,
};
use crate::vm::VirtualMachine;

/// Convert a string or number to a floating point number, if possible.
#[pyclass(name = "float")]
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct PyFloat {
    value: f64,
}

impl PyFloat {
    pub fn to_f64(self) -> f64 {
        self.value
    }
}

impl PyValue for PyFloat {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.float_type()
    }
}

impl IntoPyObject for f64 {
    fn into_pyobject(self, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.new_float(self))
    }
}
impl IntoPyObject for f32 {
    fn into_pyobject(self, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.new_float(f64::from(self)))
    }
}

impl From<f64> for PyFloat {
    fn from(value: f64) -> Self {
        PyFloat { value }
    }
}

pub fn try_float(value: &PyObjectRef, vm: &VirtualMachine) -> PyResult<Option<f64>> {
    Ok(if objtype::isinstance(&value, &vm.ctx.float_type()) {
        Some(get_value(&value))
    } else if objtype::isinstance(&value, &vm.ctx.int_type()) {
        Some(objint::get_float_value(&value, vm)?)
    } else {
        None
    })
}

macro_rules! impl_try_from_object_float {
    ($($t:ty),*) => {
        $(impl TryFromObject for $t {
            fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
                PyFloatRef::try_from_object(vm, obj).map(|f| f.to_f64() as $t)
            }
        })*
    };
}

impl_try_from_object_float!(f32, f64);

fn inner_div(v1: f64, v2: f64, vm: &VirtualMachine) -> PyResult<f64> {
    if v2 != 0.0 {
        Ok(v1 / v2)
    } else {
        Err(vm.new_zero_division_error("float division by zero".to_string()))
    }
}

fn inner_mod(v1: f64, v2: f64, vm: &VirtualMachine) -> PyResult<f64> {
    if v2 != 0.0 {
        Ok(v1 % v2)
    } else {
        Err(vm.new_zero_division_error("float mod by zero".to_string()))
    }
}

fn try_to_bigint(value: f64, vm: &VirtualMachine) -> PyResult<BigInt> {
    match value.to_bigint() {
        Some(int) => Ok(int),
        None => {
            if value.is_infinite() {
                Err(vm.new_overflow_error(
                    "OverflowError: cannot convert float infinity to integer".to_string(),
                ))
            } else if value.is_nan() {
                Err(vm
                    .new_value_error("ValueError: cannot convert float NaN to integer".to_string()))
            } else {
                // unreachable unless BigInt has a bug
                unreachable!(
                    "A finite float value failed to be converted to bigint: {}",
                    value
                )
            }
        }
    }
}

fn inner_floordiv(v1: f64, v2: f64, vm: &VirtualMachine) -> PyResult<f64> {
    if v2 != 0.0 {
        Ok((v1 / v2).floor())
    } else {
        Err(vm.new_zero_division_error("float floordiv by zero".to_string()))
    }
}

fn inner_divmod(v1: f64, v2: f64, vm: &VirtualMachine) -> PyResult<(f64, f64)> {
    if v2 != 0.0 {
        Ok(((v1 / v2).floor(), v1 % v2))
    } else {
        Err(vm.new_zero_division_error("float divmod()".to_string()))
    }
}

fn inner_lt_int(value: f64, other_int: &BigInt) -> bool {
    match (value.to_bigint(), other_int.to_f64()) {
        (Some(self_int), Some(other_float)) => value < other_float || self_int < *other_int,
        // finite float, other_int too big for float,
        // the result depends only on other_int’s sign
        (Some(_), None) => other_int.is_positive(),
        // infinite float must be bigger or lower than any int, depending on its sign
        _ if value.is_infinite() => value.is_sign_negative(),
        // NaN, always false
        _ => false,
    }
}

fn inner_gt_int(value: f64, other_int: &BigInt) -> bool {
    match (value.to_bigint(), other_int.to_f64()) {
        (Some(self_int), Some(other_float)) => value > other_float || self_int > *other_int,
        // finite float, other_int too big for float,
        // the result depends only on other_int’s sign
        (Some(_), None) => other_int.is_negative(),
        // infinite float must be bigger or lower than any int, depending on its sign
        _ if value.is_infinite() => value.is_sign_positive(),
        // NaN, always false
        _ => false,
    }
}

pub fn float_pow(v1: f64, v2: f64, vm: &VirtualMachine) -> PyResult {
    if v1.is_zero() {
        let msg = format!("{} cannot be raised to a negative power", v1);
        Err(vm.new_zero_division_error(msg))
    } else {
        v1.powf(v2).into_pyobject(vm)
    }
}

#[pyimpl]
#[allow(clippy::trivially_copy_pass_by_ref)]
impl PyFloat {
    #[pyslot(new)]
    fn tp_new(
        cls: PyClassRef,
        arg: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyFloatRef> {
        let float_val = match arg {
            OptionalArg::Present(val) => to_float(vm, &val),
            OptionalArg::Missing => Ok(0f64),
        };
        PyFloat::from(float_val?).into_ref_with_type(vm, cls)
    }

    fn float_eq(&self, other: PyObjectRef) -> bool {
        let other = get_value(&other);
        self.value == other
    }

    fn int_eq(&self, other: PyObjectRef) -> bool {
        let other_int = objint::get_value(&other);
        let value = self.value;
        if let (Some(self_int), Some(other_float)) = (value.to_bigint(), other_int.to_f64()) {
            value == other_float && self_int == *other_int
        } else {
            false
        }
    }

    #[pymethod(name = "__eq__")]
    fn eq(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        let result = if objtype::isinstance(&other, &vm.ctx.float_type()) {
            self.float_eq(other)
        } else if objtype::isinstance(&other, &vm.ctx.int_type()) {
            self.int_eq(other)
        } else {
            return vm.ctx.not_implemented();
        };
        vm.ctx.new_bool(result)
    }

    #[pymethod(name = "__ne__")]
    fn ne(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        let result = if objtype::isinstance(&other, &vm.ctx.float_type()) {
            !self.float_eq(other)
        } else if objtype::isinstance(&other, &vm.ctx.int_type()) {
            !self.int_eq(other)
        } else {
            return vm.ctx.not_implemented();
        };
        vm.ctx.new_bool(result)
    }

    #[pymethod(name = "__lt__")]
    fn lt(&self, i2: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        let v1 = self.value;
        if objtype::isinstance(&i2, &vm.ctx.float_type()) {
            vm.ctx.new_bool(v1 < get_value(&i2))
        } else if objtype::isinstance(&i2, &vm.ctx.int_type()) {
            let other_int = objint::get_value(&i2);

            vm.ctx.new_bool(inner_lt_int(self.value, other_int))
        } else {
            vm.ctx.not_implemented()
        }
    }

    #[pymethod(name = "__le__")]
    fn le(&self, i2: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        let v1 = self.value;
        if objtype::isinstance(&i2, &vm.ctx.float_type()) {
            vm.ctx.new_bool(v1 <= get_value(&i2))
        } else if objtype::isinstance(&i2, &vm.ctx.int_type()) {
            let other_int = objint::get_value(&i2);

            let result = if let (Some(self_int), Some(other_float)) =
                (self.value.to_bigint(), other_int.to_f64())
            {
                self.value <= other_float && self_int <= *other_int
            } else {
                // certainly not equal, forward to inner_lt_int
                inner_lt_int(self.value, other_int)
            };

            vm.ctx.new_bool(result)
        } else {
            vm.ctx.not_implemented()
        }
    }

    #[pymethod(name = "__gt__")]
    fn gt(&self, i2: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        let v1 = self.value;
        if objtype::isinstance(&i2, &vm.ctx.float_type()) {
            vm.ctx.new_bool(v1 > get_value(&i2))
        } else if objtype::isinstance(&i2, &vm.ctx.int_type()) {
            let other_int = objint::get_value(&i2);

            vm.ctx.new_bool(inner_gt_int(self.value, other_int))
        } else {
            vm.ctx.not_implemented()
        }
    }

    #[pymethod(name = "__ge__")]
    fn ge(&self, i2: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        let v1 = self.value;
        if objtype::isinstance(&i2, &vm.ctx.float_type()) {
            vm.ctx.new_bool(v1 >= get_value(&i2))
        } else if objtype::isinstance(&i2, &vm.ctx.int_type()) {
            let other_int = objint::get_value(&i2);

            let result = if let (Some(self_int), Some(other_float)) =
                (self.value.to_bigint(), other_int.to_f64())
            {
                self.value >= other_float && self_int >= *other_int
            } else {
                // certainly not equal, forward to inner_gt_int
                inner_gt_int(self.value, other_int)
            };

            vm.ctx.new_bool(result)
        } else {
            vm.ctx.not_implemented()
        }
    }

    #[pymethod(name = "__abs__")]
    fn abs(&self, _vm: &VirtualMachine) -> f64 {
        self.value.abs()
    }

    #[pymethod(name = "__add__")]
    fn add(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        try_float(&other, vm)?.map_or_else(
            || Ok(vm.ctx.not_implemented()),
            |other| (self.value + other).into_pyobject(vm),
        )
    }

    #[pymethod(name = "__radd__")]
    fn radd(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.add(other, vm)
    }

    #[pymethod(name = "__bool__")]
    fn bool(&self, _vm: &VirtualMachine) -> bool {
        self.value != 0.0
    }

    #[pymethod(name = "__divmod__")]
    fn divmod(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        try_float(&other, vm)?.map_or_else(
            || Ok(vm.ctx.not_implemented()),
            |other| {
                let (r1, r2) = inner_divmod(self.value, other, vm)?;
                Ok(vm
                    .ctx
                    .new_tuple(vec![vm.ctx.new_float(r1), vm.ctx.new_float(r2)]))
            },
        )
    }

    #[pymethod(name = "__rdivmod__")]
    fn rdivmod(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        try_float(&other, vm)?.map_or_else(
            || Ok(vm.ctx.not_implemented()),
            |other| {
                let (r1, r2) = inner_divmod(other, self.value, vm)?;
                Ok(vm
                    .ctx
                    .new_tuple(vec![vm.ctx.new_float(r1), vm.ctx.new_float(r2)]))
            },
        )
    }

    #[pymethod(name = "__floordiv__")]
    fn floordiv(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        try_float(&other, vm)?.map_or_else(
            || Ok(vm.ctx.not_implemented()),
            |other| inner_floordiv(self.value, other, vm)?.into_pyobject(vm),
        )
    }

    #[pymethod(name = "__rfloordiv__")]
    fn rfloordiv(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        try_float(&other, vm)?.map_or_else(
            || Ok(vm.ctx.not_implemented()),
            |other| inner_floordiv(other, self.value, vm)?.into_pyobject(vm),
        )
    }

    #[pymethod(name = "__mod__")]
    fn mod_(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        try_float(&other, vm)?.map_or_else(
            || Ok(vm.ctx.not_implemented()),
            |other| inner_mod(self.value, other, vm)?.into_pyobject(vm),
        )
    }

    #[pymethod(name = "__rmod__")]
    fn rmod(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        try_float(&other, vm)?.map_or_else(
            || Ok(vm.ctx.not_implemented()),
            |other| inner_mod(other, self.value, vm)?.into_pyobject(vm),
        )
    }

    #[pymethod(name = "__pos__")]
    fn pos(&self, _vm: &VirtualMachine) -> f64 {
        self.value
    }

    #[pymethod(name = "__neg__")]
    fn neg(&self, _vm: &VirtualMachine) -> f64 {
        -self.value
    }

    #[pymethod(name = "__pow__")]
    fn pow(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        try_float(&other, vm)?.map_or_else(
            || Ok(vm.ctx.not_implemented()),
            |other| float_pow(self.value, other, vm),
        )
    }

    #[pymethod(name = "__rpow__")]
    fn rpow(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        try_float(&other, vm)?.map_or_else(
            || Ok(vm.ctx.not_implemented()),
            |other| float_pow(other, self.value, vm),
        )
    }

    #[pymethod(name = "__sub__")]
    fn sub(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        try_float(&other, vm)?.map_or_else(
            || Ok(vm.ctx.not_implemented()),
            |other| (self.value - other).into_pyobject(vm),
        )
    }

    #[pymethod(name = "__rsub__")]
    fn rsub(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        try_float(&other, vm)?.map_or_else(
            || Ok(vm.ctx.not_implemented()),
            |other| (other - self.value).into_pyobject(vm),
        )
    }

    #[pymethod(name = "__repr__")]
    fn repr(&self, vm: &VirtualMachine) -> String {
        if self.is_integer(vm) {
            format!("{:.1?}", self.value)
        } else {
            self.value.to_string()
        }
    }

    #[pymethod(name = "__truediv__")]
    fn truediv(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        try_float(&other, vm)?.map_or_else(
            || Ok(vm.ctx.not_implemented()),
            |other| inner_div(self.value, other, vm)?.into_pyobject(vm),
        )
    }

    #[pymethod(name = "__rtruediv__")]
    fn rtruediv(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        try_float(&other, vm)?.map_or_else(
            || Ok(vm.ctx.not_implemented()),
            |other| inner_div(other, self.value, vm)?.into_pyobject(vm),
        )
    }

    #[pymethod(name = "__mul__")]
    fn mul(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        try_float(&other, vm)?.map_or_else(
            || Ok(vm.ctx.not_implemented()),
            |other| (self.value * other).into_pyobject(vm),
        )
    }

    #[pymethod(name = "__rmul__")]
    fn rmul(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.mul(other, vm)
    }

    #[pymethod(name = "__trunc__")]
    fn trunc(&self, vm: &VirtualMachine) -> PyResult<BigInt> {
        try_to_bigint(self.value, vm)
    }

    #[pymethod(name = "__round__")]
    fn round(&self, ndigits: OptionalArg<PyObjectRef>, vm: &VirtualMachine) -> PyResult {
        let ndigits = match ndigits {
            OptionalArg::Missing => None,
            OptionalArg::Present(ref value) => {
                if !vm.get_none().is(value) {
                    if !objtype::isinstance(value, &vm.ctx.int_type()) {
                        return Err(vm.new_type_error(format!(
                            "'{}' object cannot be interpreted as an integer",
                            value.class().name
                        )));
                    };
                    // Only accept int type ndigits
                    let ndigits = objint::get_value(value);
                    Some(ndigits)
                } else {
                    None
                }
            }
        };
        if let Some(ndigits) = ndigits {
            if ndigits.is_zero() {
                let fract = self.value.fract();
                let value = if (fract.abs() - 0.5).abs() < std::f64::EPSILON {
                    if self.value.trunc() % 2.0 == 0.0 {
                        self.value - fract
                    } else {
                        self.value + fract
                    }
                } else {
                    self.value.round()
                };
                Ok(vm.ctx.new_float(value))
            } else {
                let ndigits = match ndigits {
                    ndigits if *ndigits > i32::max_value().to_bigint().unwrap() => i32::max_value(),
                    ndigits if *ndigits < i32::min_value().to_bigint().unwrap() => i32::min_value(),
                    _ => ndigits.to_i32().unwrap(),
                };
                if (self.value > 1e+16_f64 && ndigits >= 0i32) || ndigits > 16i32 {
                    return Ok(vm.ctx.new_float(self.value));
                }
                if ndigits >= 0i32 {
                    return Ok(vm.ctx.new_float(
                        (self.value * pow(10.0, ndigits as usize)).round()
                            / pow(10.0, ndigits as usize),
                    ));
                } else {
                    let result = (self.value / pow(10.0, (-ndigits) as usize)).round()
                        * pow(10.0, (-ndigits) as usize);
                    if result.is_nan() {
                        return Ok(vm.ctx.new_float(0.0));
                    }
                    return Ok(vm.ctx.new_float(result));
                }
            }
        } else {
            let fract = self.value.fract();
            let value = if (fract.abs() - 0.5).abs() < std::f64::EPSILON {
                if self.value.trunc() % 2.0 == 0.0 {
                    self.value - fract
                } else {
                    self.value + fract
                }
            } else {
                self.value.round()
            };
            let int = try_to_bigint(value, vm)?;
            Ok(vm.ctx.new_int(int))
        }
    }

    #[pymethod(name = "__int__")]
    fn int(&self, vm: &VirtualMachine) -> PyResult<BigInt> {
        self.trunc(vm)
    }

    #[pymethod(name = "__float__")]
    fn float(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyFloatRef {
        zelf
    }

    #[pymethod(name = "__hash__")]
    fn hash(&self, _vm: &VirtualMachine) -> pyhash::PyHash {
        pyhash::hash_float(self.value)
    }

    #[pyproperty(name = "real")]
    fn real(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyFloatRef {
        zelf
    }

    #[pyproperty(name = "imag")]
    fn imag(&self, _vm: &VirtualMachine) -> f64 {
        0.0f64
    }

    #[pymethod(name = "conjugate")]
    fn conjugate(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyFloatRef {
        zelf
    }

    #[pymethod(name = "is_integer")]
    fn is_integer(&self, _vm: &VirtualMachine) -> bool {
        let v = self.value;
        (v - v.round()).abs() < std::f64::EPSILON
    }

    #[pymethod(name = "as_integer_ratio")]
    fn as_integer_ratio(&self, vm: &VirtualMachine) -> PyResult {
        let value = self.value;
        if value.is_infinite() {
            return Err(
                vm.new_overflow_error("cannot convert Infinity to integer ratio".to_string())
            );
        }
        if value.is_nan() {
            return Err(vm.new_value_error("cannot convert NaN to integer ratio".to_string()));
        }

        let ratio = Ratio::from_float(value).unwrap();
        let numer = vm.ctx.new_int(ratio.numer().clone());
        let denom = vm.ctx.new_int(ratio.denom().clone());
        Ok(vm.ctx.new_tuple(vec![numer, denom]))
    }

    #[pymethod]
    fn fromhex(repr: PyStringRef, vm: &VirtualMachine) -> PyResult<f64> {
        hexf_parse::parse_hexf64(repr.as_str(), false).or_else(|_| match repr.as_str() {
            "nan" => Ok(std::f64::NAN),
            "inf" => Ok(std::f64::INFINITY),
            "-inf" => Ok(std::f64::NEG_INFINITY),
            _ => Err(vm.new_value_error("invalid hexadecimal floating-point string".to_string())),
        })
    }

    #[pymethod]
    fn hex(&self, _vm: &VirtualMachine) -> String {
        to_hex(self.value)
    }
}

fn to_float(vm: &VirtualMachine, obj: &PyObjectRef) -> PyResult<f64> {
    let value = if objtype::isinstance(&obj, &vm.ctx.float_type()) {
        get_value(&obj)
    } else if objtype::isinstance(&obj, &vm.ctx.int_type()) {
        objint::get_float_value(&obj, vm)?
    } else if objtype::isinstance(&obj, &vm.ctx.str_type()) {
        match lexical::try_parse(objstr::get_value(&obj).trim()) {
            Ok(f) => f,
            Err(_) => {
                let arg_repr = vm.to_pystr(obj)?;
                return Err(vm.new_value_error(format!(
                    "could not convert string to float: '{}'",
                    arg_repr
                )));
            }
        }
    } else if objtype::isinstance(&obj, &vm.ctx.bytes_type()) {
        match lexical::try_parse(objbytes::get_value(&obj).as_slice()) {
            Ok(f) => f,
            Err(_) => {
                let arg_repr = vm.to_pystr(obj)?;
                return Err(vm.new_value_error(format!(
                    "could not convert string to float: '{}'",
                    arg_repr
                )));
            }
        }
    } else {
        return Err(vm.new_type_error(format!("can't convert {} to float", obj.class().name)));
    };
    Ok(value)
}

fn to_hex(value: f64) -> String {
    let (mantissa, exponent, sign) = value.integer_decode();
    let sign_fmt = if sign < 0 { "-" } else { "" };
    match value {
        value if value.is_zero() => format!("{}0x0.0p+0", sign_fmt),
        value if value.is_infinite() => format!("{}inf", sign_fmt),
        value if value.is_nan() => "nan".to_string(),
        _ => {
            const BITS: i16 = 52;
            const FRACT_MASK: u64 = 0xf_ffff_ffff_ffff;
            format!(
                "{}0x{:x}.{:013x}p{:+}",
                sign_fmt,
                mantissa >> BITS,
                mantissa & FRACT_MASK,
                exponent + BITS
            )
        }
    }
}

#[test]
fn test_to_hex() {
    use rand::Rng;
    for _ in 0..20000 {
        let bytes = rand::thread_rng().gen::<[u64; 1]>();
        let f = f64::from_bits(bytes[0]);
        if !f.is_finite() {
            continue;
        }
        let hex = to_hex(f);
        // println!("{} -> {}", f, hex);
        let roundtrip = hexf_parse::parse_hexf64(&hex, false).unwrap();
        // println!("  -> {}", roundtrip);
        assert!(f == roundtrip, "{} {} {}", f, hex, roundtrip);
    }
}

pub fn ufrexp(value: f64) -> (f64, i32) {
    if 0.0 == value {
        (0.0, 0i32)
    } else {
        let bits = value.to_bits();
        let exponent: i32 = ((bits >> 52) & 0x7ff) as i32 - 1022;
        let mantissa_bits = bits & (0x000f_ffff_ffff_ffff) | (1022 << 52);
        (f64::from_bits(mantissa_bits), exponent)
    }
}

pub type PyFloatRef = PyRef<PyFloat>;

// Retrieve inner float value:
pub fn get_value(obj: &PyObjectRef) -> f64 {
    obj.payload::<PyFloat>().unwrap().value
}

fn make_float(vm: &VirtualMachine, obj: &PyObjectRef) -> PyResult<f64> {
    if objtype::isinstance(obj, &vm.ctx.float_type()) {
        Ok(get_value(obj))
    } else {
        let method = vm.get_method_or_type_error(obj.clone(), "__float__", || {
            format!(
                "float() argument must be a string or a number, not '{}'",
                obj.class().name
            )
        })?;
        let result = vm.invoke(&method, vec![])?;
        Ok(get_value(&result))
    }
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub struct IntoPyFloat {
    value: f64,
}

impl IntoPyFloat {
    pub fn to_f64(self) -> f64 {
        self.value
    }
}

impl TryFromObject for IntoPyFloat {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        Ok(IntoPyFloat {
            value: make_float(vm, &obj)?,
        })
    }
}

#[rustfmt::skip] // to avoid line splitting
pub fn init(context: &PyContext) {
    PyFloat::extend_class(context, &context.types.float_type);
}
