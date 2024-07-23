// spell-checker:ignore numer denom

use super::{
    try_bigint_to_f64, PyByteArray, PyBytes, PyInt, PyIntRef, PyStr, PyStrRef, PyType, PyTypeRef,
};
use crate::{
    class::PyClassImpl,
    common::{float_ops, hash},
    convert::{IntoPyException, ToPyObject, ToPyResult},
    function::{
        ArgBytesLike, OptionalArg, OptionalOption,
        PyArithmeticValue::{self, *},
        PyComparisonValue,
    },
    protocol::PyNumberMethods,
    types::{AsNumber, Callable, Comparable, Constructor, Hashable, PyComparisonOp, Representable},
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult,
    TryFromBorrowedObject, TryFromObject, VirtualMachine,
};
use malachite_bigint::{BigInt, ToBigInt};
use num_complex::Complex64;
use num_traits::{Signed, ToPrimitive, Zero};
use rustpython_common::int::float_to_ratio;
use rustpython_format::FormatSpec;

#[pyclass(module = false, name = "float")]
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct PyFloat {
    value: f64,
}

impl PyFloat {
    pub fn to_f64(&self) -> f64 {
        self.value
    }
}

impl PyPayload for PyFloat {
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.float_type
    }
}

impl ToPyObject for f64 {
    fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_float(self).into()
    }
}
impl ToPyObject for f32 {
    fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_float(f64::from(self)).into()
    }
}

impl From<f64> for PyFloat {
    fn from(value: f64) -> Self {
        PyFloat { value }
    }
}

pub(crate) fn to_op_float(obj: &PyObject, vm: &VirtualMachine) -> PyResult<Option<f64>> {
    let v = if let Some(float) = obj.payload_if_subclass::<PyFloat>(vm) {
        Some(float.value)
    } else if let Some(int) = obj.payload_if_subclass::<PyInt>(vm) {
        Some(try_bigint_to_f64(int.as_bigint(), vm)?)
    } else {
        None
    };
    Ok(v)
}

macro_rules! impl_try_from_object_float {
    ($($t:ty),*) => {
        $(impl TryFromObject for $t {
            fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
                PyRef::<PyFloat>::try_from_object(vm, obj).map(|f| f.to_f64() as $t)
            }
        })*
    };
}

impl_try_from_object_float!(f32, f64);

fn inner_div(v1: f64, v2: f64, vm: &VirtualMachine) -> PyResult<f64> {
    float_ops::div(v1, v2)
        .ok_or_else(|| vm.new_zero_division_error("float division by zero".to_owned()))
}

fn inner_mod(v1: f64, v2: f64, vm: &VirtualMachine) -> PyResult<f64> {
    float_ops::mod_(v1, v2)
        .ok_or_else(|| vm.new_zero_division_error("float mod by zero".to_owned()))
}

pub fn try_to_bigint(value: f64, vm: &VirtualMachine) -> PyResult<BigInt> {
    match value.to_bigint() {
        Some(int) => Ok(int),
        None => {
            if value.is_infinite() {
                Err(vm.new_overflow_error(
                    "OverflowError: cannot convert float infinity to integer".to_owned(),
                ))
            } else if value.is_nan() {
                Err(vm
                    .new_value_error("ValueError: cannot convert float NaN to integer".to_owned()))
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
    float_ops::floordiv(v1, v2)
        .ok_or_else(|| vm.new_zero_division_error("float floordiv by zero".to_owned()))
}

fn inner_divmod(v1: f64, v2: f64, vm: &VirtualMachine) -> PyResult<(f64, f64)> {
    float_ops::divmod(v1, v2).ok_or_else(|| vm.new_zero_division_error("float divmod()".to_owned()))
}

pub fn float_pow(v1: f64, v2: f64, vm: &VirtualMachine) -> PyResult {
    if v1.is_zero() && v2.is_sign_negative() {
        let msg = format!("{v1} cannot be raised to a negative power");
        Err(vm.new_zero_division_error(msg))
    } else if v1.is_sign_negative() && (v2.floor() - v2).abs() > f64::EPSILON {
        let v1 = Complex64::new(v1, 0.);
        let v2 = Complex64::new(v2, 0.);
        Ok(v1.powc(v2).to_pyobject(vm))
    } else {
        Ok(v1.powf(v2).to_pyobject(vm))
    }
}

impl Constructor for PyFloat {
    type Args = OptionalArg<PyObjectRef>;

    fn py_new(cls: PyTypeRef, arg: Self::Args, vm: &VirtualMachine) -> PyResult {
        let float_val = match arg {
            OptionalArg::Missing => 0.0,
            OptionalArg::Present(val) => {
                if cls.is(vm.ctx.types.float_type) && val.class().is(vm.ctx.types.float_type) {
                    return Ok(val);
                }

                if let Some(f) = val.try_float_opt(vm) {
                    f?.value
                } else {
                    float_from_string(val, vm)?
                }
            }
        };
        PyFloat::from(float_val)
            .into_ref_with_type(vm, cls)
            .map(Into::into)
    }
}

fn float_from_string(val: PyObjectRef, vm: &VirtualMachine) -> PyResult<f64> {
    let (bytearray, buffer, buffer_lock);
    let b = if let Some(s) = val.payload_if_subclass::<PyStr>(vm) {
        s.as_str().trim().as_bytes()
    } else if let Some(bytes) = val.payload_if_subclass::<PyBytes>(vm) {
        bytes.as_bytes()
    } else if let Some(buf) = val.payload_if_subclass::<PyByteArray>(vm) {
        bytearray = buf.borrow_buf();
        &*bytearray
    } else if let Ok(b) = ArgBytesLike::try_from_borrowed_object(vm, &val) {
        buffer = b;
        buffer_lock = buffer.borrow_buf();
        &*buffer_lock
    } else {
        return Err(vm.new_type_error(format!(
            "float() argument must be a string or a number, not '{}'",
            val.class().name()
        )));
    };
    crate::literal::float::parse_bytes(b).ok_or_else(|| {
        val.repr(vm)
            .map(|repr| vm.new_value_error(format!("could not convert string to float: {repr}")))
            .unwrap_or_else(|e| e)
    })
}

#[pyclass(
    flags(BASETYPE),
    with(Comparable, Hashable, Constructor, AsNumber, Representable)
)]
impl PyFloat {
    #[pymethod(magic)]
    fn format(&self, spec: PyStrRef, vm: &VirtualMachine) -> PyResult<String> {
        FormatSpec::parse(spec.as_str())
            .and_then(|format_spec| format_spec.format_float(self.value))
            .map_err(|err| err.into_pyexception(vm))
    }

    #[pystaticmethod(magic)]
    fn getformat(spec: PyStrRef, vm: &VirtualMachine) -> PyResult<String> {
        if !matches!(spec.as_str(), "double" | "float") {
            return Err(vm.new_value_error(
                "__getformat__() argument 1 must be 'double' or 'float'".to_owned(),
            ));
        }

        const BIG_ENDIAN: bool = cfg!(target_endian = "big");

        Ok(if BIG_ENDIAN {
            "IEEE, big-endian"
        } else {
            "IEEE, little-endian"
        }
        .to_owned())
    }

    #[pymethod(magic)]
    fn abs(&self) -> f64 {
        self.value.abs()
    }

    #[inline]
    fn simple_op<F>(
        &self,
        other: PyObjectRef,
        op: F,
        vm: &VirtualMachine,
    ) -> PyResult<PyArithmeticValue<f64>>
    where
        F: Fn(f64, f64) -> PyResult<f64>,
    {
        to_op_float(&other, vm)?.map_or_else(
            || Ok(NotImplemented),
            |other| Ok(Implemented(op(self.value, other)?)),
        )
    }

    #[inline]
    fn complex_op<F>(&self, other: PyObjectRef, op: F, vm: &VirtualMachine) -> PyResult
    where
        F: Fn(f64, f64) -> PyResult,
    {
        to_op_float(&other, vm)?.map_or_else(
            || Ok(vm.ctx.not_implemented()),
            |other| op(self.value, other),
        )
    }

    #[inline]
    fn tuple_op<F>(
        &self,
        other: PyObjectRef,
        op: F,
        vm: &VirtualMachine,
    ) -> PyResult<PyArithmeticValue<(f64, f64)>>
    where
        F: Fn(f64, f64) -> PyResult<(f64, f64)>,
    {
        to_op_float(&other, vm)?.map_or_else(
            || Ok(NotImplemented),
            |other| Ok(Implemented(op(self.value, other)?)),
        )
    }

    #[pymethod(name = "__radd__")]
    #[pymethod(magic)]
    fn add(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyArithmeticValue<f64>> {
        self.simple_op(other, |a, b| Ok(a + b), vm)
    }

    #[pymethod(magic)]
    fn bool(&self) -> bool {
        self.value != 0.0
    }

    #[pymethod(magic)]
    fn divmod(
        &self,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyArithmeticValue<(f64, f64)>> {
        self.tuple_op(other, |a, b| inner_divmod(a, b, vm), vm)
    }

    #[pymethod(magic)]
    fn rdivmod(
        &self,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyArithmeticValue<(f64, f64)>> {
        self.tuple_op(other, |a, b| inner_divmod(b, a, vm), vm)
    }

    #[pymethod(magic)]
    fn floordiv(
        &self,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyArithmeticValue<f64>> {
        self.simple_op(other, |a, b| inner_floordiv(a, b, vm), vm)
    }

    #[pymethod(magic)]
    fn rfloordiv(
        &self,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyArithmeticValue<f64>> {
        self.simple_op(other, |a, b| inner_floordiv(b, a, vm), vm)
    }

    #[pymethod(name = "__mod__")]
    fn mod_(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyArithmeticValue<f64>> {
        self.simple_op(other, |a, b| inner_mod(a, b, vm), vm)
    }

    #[pymethod(magic)]
    fn rmod(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyArithmeticValue<f64>> {
        self.simple_op(other, |a, b| inner_mod(b, a, vm), vm)
    }

    #[pymethod(magic)]
    fn pos(&self) -> f64 {
        self.value
    }

    #[pymethod(magic)]
    fn neg(&self) -> f64 {
        -self.value
    }

    #[pymethod(magic)]
    fn pow(
        &self,
        other: PyObjectRef,
        mod_val: OptionalOption<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        if mod_val.flatten().is_some() {
            Err(vm.new_type_error("floating point pow() does not accept a 3rd argument".to_owned()))
        } else {
            self.complex_op(other, |a, b| float_pow(a, b, vm), vm)
        }
    }

    #[pymethod(magic)]
    fn rpow(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.complex_op(other, |a, b| float_pow(b, a, vm), vm)
    }

    #[pymethod(magic)]
    fn sub(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyArithmeticValue<f64>> {
        self.simple_op(other, |a, b| Ok(a - b), vm)
    }

    #[pymethod(magic)]
    fn rsub(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyArithmeticValue<f64>> {
        self.simple_op(other, |a, b| Ok(b - a), vm)
    }

    #[pymethod(magic)]
    fn truediv(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyArithmeticValue<f64>> {
        self.simple_op(other, |a, b| inner_div(a, b, vm), vm)
    }

    #[pymethod(magic)]
    fn rtruediv(
        &self,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyArithmeticValue<f64>> {
        self.simple_op(other, |a, b| inner_div(b, a, vm), vm)
    }

    #[pymethod(name = "__rmul__")]
    #[pymethod(magic)]
    fn mul(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyArithmeticValue<f64>> {
        self.simple_op(other, |a, b| Ok(a * b), vm)
    }

    #[pymethod(magic)]
    fn trunc(&self, vm: &VirtualMachine) -> PyResult<BigInt> {
        try_to_bigint(self.value, vm)
    }

    #[pymethod(magic)]
    fn floor(&self, vm: &VirtualMachine) -> PyResult<BigInt> {
        try_to_bigint(self.value.floor(), vm)
    }

    #[pymethod(magic)]
    fn ceil(&self, vm: &VirtualMachine) -> PyResult<BigInt> {
        try_to_bigint(self.value.ceil(), vm)
    }

    #[pymethod(magic)]
    fn round(&self, ndigits: OptionalOption<PyIntRef>, vm: &VirtualMachine) -> PyResult {
        let ndigits = ndigits.flatten();
        let value = if let Some(ndigits) = ndigits {
            let ndigits = ndigits.as_bigint();
            let ndigits = match ndigits.to_i32() {
                Some(n) => n,
                None if ndigits.is_positive() => i32::MAX,
                None => i32::MIN,
            };
            let float = float_ops::round_float_digits(self.value, ndigits).ok_or_else(|| {
                vm.new_overflow_error("overflow occurred during round".to_owned())
            })?;
            vm.ctx.new_float(float).into()
        } else {
            let fract = self.value.fract();
            let value = if (fract.abs() - 0.5).abs() < f64::EPSILON {
                if self.value.trunc() % 2.0 == 0.0 {
                    self.value - fract
                } else {
                    self.value + fract
                }
            } else {
                self.value.round()
            };
            let int = try_to_bigint(value, vm)?;
            vm.ctx.new_int(int).into()
        };
        Ok(value)
    }

    #[pymethod(magic)]
    fn int(&self, vm: &VirtualMachine) -> PyResult<BigInt> {
        self.trunc(vm)
    }

    #[pymethod(magic)]
    fn float(zelf: PyRef<Self>) -> PyRef<Self> {
        zelf
    }

    #[pygetset]
    fn real(zelf: PyRef<Self>) -> PyRef<Self> {
        zelf
    }

    #[pygetset]
    fn imag(&self) -> f64 {
        0.0f64
    }

    #[pymethod]
    fn conjugate(zelf: PyRef<Self>) -> PyRef<Self> {
        zelf
    }

    #[pymethod]
    fn is_integer(&self) -> bool {
        crate::literal::float::is_integer(self.value)
    }

    #[pymethod]
    fn as_integer_ratio(&self, vm: &VirtualMachine) -> PyResult<(PyIntRef, PyIntRef)> {
        let value = self.value;

        float_to_ratio(value)
            .map(|(numer, denom)| (vm.ctx.new_bigint(&numer), vm.ctx.new_bigint(&denom)))
            .ok_or_else(|| {
                if value.is_infinite() {
                    vm.new_overflow_error("cannot convert Infinity to integer ratio".to_owned())
                } else if value.is_nan() {
                    vm.new_value_error("cannot convert NaN to integer ratio".to_owned())
                } else {
                    unreachable!("finite float must able to convert to integer ratio")
                }
            })
    }

    #[pyclassmethod]
    fn fromhex(cls: PyTypeRef, string: PyStrRef, vm: &VirtualMachine) -> PyResult {
        let result = crate::literal::float::from_hex(string.as_str().trim()).ok_or_else(|| {
            vm.new_value_error("invalid hexadecimal floating-point string".to_owned())
        })?;
        PyType::call(&cls, vec![vm.ctx.new_float(result).into()].into(), vm)
    }

    #[pymethod]
    fn hex(&self) -> String {
        crate::literal::float::to_hex(self.value)
    }

    #[pymethod(magic)]
    fn getnewargs(&self, vm: &VirtualMachine) -> PyObjectRef {
        (self.value,).to_pyobject(vm)
    }
}

impl Comparable for PyFloat {
    fn cmp(
        zelf: &Py<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        let ret = if let Some(other) = other.payload_if_subclass::<PyFloat>(vm) {
            zelf.value
                .partial_cmp(&other.value)
                .map_or_else(|| op == PyComparisonOp::Ne, |ord| op.eval_ord(ord))
        } else if let Some(other) = other.payload_if_subclass::<PyInt>(vm) {
            let a = zelf.to_f64();
            let b = other.as_bigint();
            match op {
                PyComparisonOp::Lt => float_ops::lt_int(a, b),
                PyComparisonOp::Le => {
                    if let (Some(a_int), Some(b_float)) = (a.to_bigint(), b.to_f64()) {
                        a <= b_float && a_int <= *b
                    } else {
                        float_ops::lt_int(a, b)
                    }
                }
                PyComparisonOp::Eq => float_ops::eq_int(a, b),
                PyComparisonOp::Ne => !float_ops::eq_int(a, b),
                PyComparisonOp::Ge => {
                    if let (Some(a_int), Some(b_float)) = (a.to_bigint(), b.to_f64()) {
                        a >= b_float && a_int >= *b
                    } else {
                        float_ops::gt_int(a, b)
                    }
                }
                PyComparisonOp::Gt => float_ops::gt_int(a, b),
            }
        } else {
            return Ok(NotImplemented);
        };
        Ok(Implemented(ret))
    }
}

impl Hashable for PyFloat {
    #[inline]
    fn hash(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<hash::PyHash> {
        Ok(hash::hash_float(zelf.to_f64()).unwrap_or_else(|| hash::hash_object_id(zelf.get_id())))
    }
}

impl AsNumber for PyFloat {
    fn as_number() -> &'static PyNumberMethods {
        static AS_NUMBER: PyNumberMethods = PyNumberMethods {
            add: Some(|a, b, vm| PyFloat::number_op(a, b, |a, b, _vm| a + b, vm)),
            subtract: Some(|a, b, vm| PyFloat::number_op(a, b, |a, b, _vm| a - b, vm)),
            multiply: Some(|a, b, vm| PyFloat::number_op(a, b, |a, b, _vm| a * b, vm)),
            remainder: Some(|a, b, vm| PyFloat::number_op(a, b, inner_mod, vm)),
            divmod: Some(|a, b, vm| PyFloat::number_op(a, b, inner_divmod, vm)),
            power: Some(|a, b, c, vm| {
                if vm.is_none(c) {
                    PyFloat::number_op(a, b, float_pow, vm)
                } else {
                    Err(vm.new_type_error(String::from(
                        "pow() 3rd argument not allowed unless all arguments are integers",
                    )))
                }
            }),
            negative: Some(|num, vm| {
                let value = PyFloat::number_downcast(num).value;
                (-value).to_pyresult(vm)
            }),
            positive: Some(|num, vm| PyFloat::number_downcast_exact(num, vm).to_pyresult(vm)),
            absolute: Some(|num, vm| {
                let value = PyFloat::number_downcast(num).value;
                value.abs().to_pyresult(vm)
            }),
            boolean: Some(|num, _vm| Ok(PyFloat::number_downcast(num).value.is_zero())),
            int: Some(|num, vm| {
                let value = PyFloat::number_downcast(num).value;
                try_to_bigint(value, vm).map(|x| PyInt::from(x).into_pyobject(vm))
            }),
            float: Some(|num, vm| Ok(PyFloat::number_downcast_exact(num, vm).into())),
            floor_divide: Some(|a, b, vm| PyFloat::number_op(a, b, inner_floordiv, vm)),
            true_divide: Some(|a, b, vm| PyFloat::number_op(a, b, inner_div, vm)),
            ..PyNumberMethods::NOT_IMPLEMENTED
        };
        &AS_NUMBER
    }

    #[inline]
    fn clone_exact(zelf: &Py<Self>, vm: &VirtualMachine) -> PyRef<Self> {
        vm.ctx.new_float(zelf.value)
    }
}

impl Representable for PyFloat {
    #[inline]
    fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
        Ok(crate::literal::float::to_string(zelf.value))
    }
}

impl PyFloat {
    fn number_op<F, R>(a: &PyObject, b: &PyObject, op: F, vm: &VirtualMachine) -> PyResult
    where
        F: FnOnce(f64, f64, &VirtualMachine) -> R,
        R: ToPyResult,
    {
        if let (Some(a), Some(b)) = (to_op_float(a, vm)?, to_op_float(b, vm)?) {
            op(a, b, vm).to_pyresult(vm)
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }
}

// Retrieve inner float value:
#[cfg(feature = "serde")]
pub(crate) fn get_value(obj: &PyObject) -> f64 {
    obj.payload::<PyFloat>().unwrap().value
}

#[rustfmt::skip] // to avoid line splitting
pub fn init(context: &Context) {
    PyFloat::extend_class(context, context.types.float_type);
}
