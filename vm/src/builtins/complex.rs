use super::{float, PyStr, PyType, PyTypeRef};
use crate::{
    atomic_func,
    class::PyClassImpl,
    convert::{ToPyObject, ToPyResult},
    function::{
        OptionalArg, OptionalOption,
        PyArithmeticValue::{self, *},
        PyComparisonValue,
    },
    identifier,
    protocol::{PyNumber, PyNumberMethods},
    types::{AsNumber, Comparable, Constructor, Hashable, PyComparisonOp},
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
};
use num_complex::Complex64;
use num_traits::Zero;
use once_cell::sync::Lazy;
use rustpython_common::{float_ops, hash};
use std::num::Wrapping;

/// Create a complex number from a real part and an optional imaginary part.
///
/// This is equivalent to (real + imag*1j) where imag defaults to 0.
#[pyclass(module = false, name = "complex")]
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct PyComplex {
    value: Complex64,
}

impl PyPayload for PyComplex {
    fn class(vm: &VirtualMachine) -> &'static Py<PyType> {
        vm.ctx.types.complex_type
    }
}

impl ToPyObject for Complex64 {
    fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        PyComplex::new_ref(self, &vm.ctx).into()
    }
}

impl From<Complex64> for PyComplex {
    fn from(value: Complex64) -> Self {
        PyComplex { value }
    }
}

impl PyObjectRef {
    /// Tries converting a python object into a complex, returns an option of whether the complex
    /// and whether the  object was a complex originally or coereced into one
    pub fn try_complex(&self, vm: &VirtualMachine) -> PyResult<Option<(Complex64, bool)>> {
        if let Some(complex) = self.payload_if_exact::<PyComplex>(vm) {
            return Ok(Some((complex.value, true)));
        }
        if let Some(method) = vm.get_method(self.clone(), identifier!(vm, __complex__)) {
            let result = vm.invoke(&method?, ())?;
            // TODO: returning strict subclasses of complex in __complex__ is deprecated
            return match result.payload::<PyComplex>() {
                Some(complex_obj) => Ok(Some((complex_obj.value, true))),
                None => Err(vm.new_type_error(format!(
                    "__complex__ returned non-complex (type '{}')",
                    result.class().name()
                ))),
            };
        }
        // `complex` does not have a `__complex__` by default, so subclasses might not either,
        // use the actual stored value in this case
        if let Some(complex) = self.payload_if_subclass::<PyComplex>(vm) {
            return Ok(Some((complex.value, true)));
        }
        if let Some(float) = self.try_float_opt(vm) {
            return Ok(Some((Complex64::new(float?.to_f64(), 0.0), false)));
        }
        Ok(None)
    }
}

pub fn init(context: &Context) {
    PyComplex::extend_class(context, context.types.complex_type);
}

fn to_op_complex(value: &PyObject, vm: &VirtualMachine) -> PyResult<Option<Complex64>> {
    let r = if let Some(complex) = value.payload_if_subclass::<PyComplex>(vm) {
        Some(complex.value)
    } else {
        float::to_op_float(value, vm)?.map(|float| Complex64::new(float, 0.0))
    };
    Ok(r)
}

fn inner_div(v1: Complex64, v2: Complex64, vm: &VirtualMachine) -> PyResult<Complex64> {
    if v2.is_zero() {
        return Err(vm.new_zero_division_error("complex division by zero".to_owned()));
    }

    Ok(v1.fdiv(v2))
}

fn inner_pow(v1: Complex64, v2: Complex64, vm: &VirtualMachine) -> PyResult<Complex64> {
    if v1.is_zero() {
        return if v2.im != 0.0 {
            let msg = format!("{v1} cannot be raised to a negative or complex power");
            Err(vm.new_zero_division_error(msg))
        } else if v2.is_zero() {
            Ok(Complex64::new(1.0, 0.0))
        } else {
            Ok(Complex64::new(0.0, 0.0))
        };
    }

    let ans = v1.powc(v2);
    if ans.is_infinite() && !(v1.is_infinite() || v2.is_infinite()) {
        Err(vm.new_overflow_error("complex exponentiation overflow".to_owned()))
    } else {
        Ok(ans)
    }
}

impl Constructor for PyComplex {
    type Args = ComplexArgs;

    fn py_new(cls: PyTypeRef, args: Self::Args, vm: &VirtualMachine) -> PyResult {
        let imag_missing = args.imag.is_missing();
        let (real, real_was_complex) = match args.real {
            OptionalArg::Missing => (Complex64::new(0.0, 0.0), false),
            OptionalArg::Present(val) => {
                let val = if cls.is(vm.ctx.types.complex_type) && imag_missing {
                    match val.downcast_exact::<PyComplex>(vm) {
                        Ok(c) => {
                            return Ok(c.into_pyref().into());
                        }
                        Err(val) => val,
                    }
                } else {
                    val
                };

                if let Some(c) = val.try_complex(vm)? {
                    c
                } else if let Some(s) = val.payload_if_subclass::<PyStr>(vm) {
                    if args.imag.is_present() {
                        return Err(vm.new_type_error(
                            "complex() can't take second arg if first is a string".to_owned(),
                        ));
                    }
                    let value = parse_str(s.as_str().trim()).ok_or_else(|| {
                        vm.new_value_error("complex() arg is a malformed string".to_owned())
                    })?;
                    return Self::from(value)
                        .into_ref_with_type(vm, cls)
                        .map(Into::into);
                } else {
                    return Err(vm.new_type_error(format!(
                        "complex() first argument must be a string or a number, not '{}'",
                        val.class().name()
                    )));
                }
            }
        };

        let (imag, imag_was_complex) = match args.imag {
            // Copy the imaginary from the real to the real of the imaginary
            // if an  imaginary argument is not passed in
            OptionalArg::Missing => (Complex64::new(real.im, 0.0), false),
            OptionalArg::Present(obj) => {
                if let Some(c) = obj.try_complex(vm)? {
                    c
                } else if obj.class().fast_issubclass(vm.ctx.types.str_type) {
                    return Err(
                        vm.new_type_error("complex() second arg can't be a string".to_owned())
                    );
                } else {
                    return Err(vm.new_type_error(format!(
                        "complex() second argument must be a number, not '{}'",
                        obj.class().name()
                    )));
                }
            }
        };

        let final_real = if imag_was_complex {
            real.re - imag.im
        } else {
            real.re
        };

        let final_imag = if real_was_complex && !imag_missing {
            imag.re + real.im
        } else {
            imag.re
        };
        let value = Complex64::new(final_real, final_imag);
        Self::from(value)
            .into_ref_with_type(vm, cls)
            .map(Into::into)
    }
}

impl PyComplex {
    pub fn new_ref(value: Complex64, ctx: &Context) -> PyRef<Self> {
        PyRef::new_ref(Self::from(value), ctx.types.complex_type.to_owned(), None)
    }

    pub fn to_complex(&self) -> Complex64 {
        self.value
    }
}

#[pyclass(flags(BASETYPE), with(Comparable, Hashable, Constructor, AsNumber))]
impl PyComplex {
    #[pymethod(magic)]
    fn complex(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyRef<PyComplex> {
        if zelf.is(vm.ctx.types.complex_type) {
            zelf
        } else {
            PyComplex::from(zelf.value).into_ref(vm)
        }
    }

    #[pygetset]
    fn real(&self) -> f64 {
        self.value.re
    }

    #[pygetset]
    fn imag(&self) -> f64 {
        self.value.im
    }

    #[pymethod(magic)]
    fn abs(&self, vm: &VirtualMachine) -> PyResult<f64> {
        let Complex64 { im, re } = self.value;
        let is_finite = im.is_finite() && re.is_finite();
        let abs_result = re.hypot(im);
        if is_finite && abs_result.is_infinite() {
            Err(vm.new_overflow_error("absolute value too large".to_string()))
        } else {
            Ok(abs_result)
        }
    }

    #[inline]
    fn op<F>(
        &self,
        other: PyObjectRef,
        op: F,
        vm: &VirtualMachine,
    ) -> PyResult<PyArithmeticValue<Complex64>>
    where
        F: Fn(Complex64, Complex64) -> PyResult<Complex64>,
    {
        to_op_complex(&other, vm)?.map_or_else(
            || Ok(NotImplemented),
            |other| Ok(Implemented(op(self.value, other)?)),
        )
    }

    #[pymethod(name = "__radd__")]
    #[pymethod(magic)]
    fn add(
        &self,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyArithmeticValue<Complex64>> {
        self.op(other, |a, b| Ok(a + b), vm)
    }

    #[pymethod(magic)]
    fn sub(
        &self,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyArithmeticValue<Complex64>> {
        self.op(other, |a, b| Ok(a - b), vm)
    }

    #[pymethod(magic)]
    fn rsub(
        &self,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyArithmeticValue<Complex64>> {
        self.op(other, |a, b| Ok(b - a), vm)
    }

    #[pymethod]
    fn conjugate(&self) -> Complex64 {
        self.value.conj()
    }

    #[pymethod(name = "__rmul__")]
    #[pymethod(magic)]
    fn mul(
        &self,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyArithmeticValue<Complex64>> {
        self.op(other, |a, b| Ok(a * b), vm)
    }

    #[pymethod(magic)]
    fn truediv(
        &self,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyArithmeticValue<Complex64>> {
        self.op(other, |a, b| inner_div(a, b, vm), vm)
    }

    #[pymethod(magic)]
    fn rtruediv(
        &self,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyArithmeticValue<Complex64>> {
        self.op(other, |a, b| inner_div(b, a, vm), vm)
    }

    #[pymethod(magic)]
    fn pos(&self) -> Complex64 {
        self.value
    }

    #[pymethod(magic)]
    fn neg(&self) -> Complex64 {
        -self.value
    }

    #[pymethod(magic)]
    fn repr(&self) -> String {
        // TODO: when you fix this, move it to rustpython_common::complex::repr and update
        //       ast/src/unparse.rs + impl Display for Constant in ast/src/constant.rs
        let Complex64 { re, im } = self.value;
        // integer => drop ., fractional => float_ops
        let mut im_part = if im.fract() == 0.0 {
            im.to_string()
        } else {
            float_ops::to_string(im)
        };
        im_part.push('j');

        // positive empty => return im_part, integer => drop ., fractional => float_ops
        let re_part = if re == 0.0 {
            if re.is_sign_positive() {
                return im_part;
            } else {
                re.to_string()
            }
        } else if re.fract() == 0.0 {
            re.to_string()
        } else {
            float_ops::to_string(re)
        };
        let mut result = String::with_capacity(
            re_part.len() + im_part.len() + 2 + im.is_sign_positive() as usize,
        );
        result.push('(');
        result.push_str(&re_part);
        if im.is_sign_positive() || im.is_nan() {
            result.push('+');
        }
        result.push_str(&im_part);
        result.push(')');
        result
    }

    #[pymethod(magic)]
    fn pow(
        &self,
        other: PyObjectRef,
        mod_val: OptionalOption<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyArithmeticValue<Complex64>> {
        if mod_val.flatten().is_some() {
            Err(vm.new_value_error("complex modulo not allowed".to_owned()))
        } else {
            self.op(other, |a, b| inner_pow(a, b, vm), vm)
        }
    }

    #[pymethod(magic)]
    fn rpow(
        &self,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyArithmeticValue<Complex64>> {
        self.op(other, |a, b| inner_pow(b, a, vm), vm)
    }

    #[pymethod(magic)]
    fn bool(&self) -> bool {
        !Complex64::is_zero(&self.value)
    }

    #[pymethod(magic)]
    fn getnewargs(&self) -> (f64, f64) {
        let Complex64 { re, im } = self.value;
        (re, im)
    }
}

impl Comparable for PyComplex {
    fn cmp(
        zelf: &crate::Py<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        op.eq_only(|| {
            let result = if let Some(other) = other.payload_if_subclass::<PyComplex>(vm) {
                if zelf.value.re.is_nan()
                    && zelf.value.im.is_nan()
                    && other.value.re.is_nan()
                    && other.value.im.is_nan()
                {
                    true
                } else {
                    zelf.value == other.value
                }
            } else {
                match float::to_op_float(other, vm) {
                    Ok(Some(other)) => zelf.value == other.into(),
                    Err(_) => false,
                    Ok(None) => return Ok(PyComparisonValue::NotImplemented),
                }
            };
            Ok(PyComparisonValue::Implemented(result))
        })
    }
}

impl Hashable for PyComplex {
    #[inline]
    fn hash(zelf: &crate::Py<Self>, _vm: &VirtualMachine) -> PyResult<hash::PyHash> {
        let value = zelf.value;

        let re_hash =
            hash::hash_float(value.re).unwrap_or_else(|| hash::hash_object_id(zelf.get_id()));

        let im_hash =
            hash::hash_float(value.im).unwrap_or_else(|| hash::hash_object_id(zelf.get_id()));

        let Wrapping(ret) = Wrapping(re_hash) + Wrapping(im_hash) * Wrapping(hash::IMAG);
        Ok(hash::fix_sentinel(ret))
    }
}

impl AsNumber for PyComplex {
    fn as_number() -> &'static PyNumberMethods {
        static AS_NUMBER: Lazy<PyNumberMethods> = Lazy::new(|| PyNumberMethods {
            add: atomic_func!(|number, other, vm| PyComplex::number_complex_op(
                number,
                other,
                |a, b| a + b,
                vm
            )),
            subtract: atomic_func!(|number, other, vm| {
                PyComplex::number_complex_op(number, other, |a, b| a - b, vm)
            }),
            multiply: atomic_func!(|number, other, vm| {
                PyComplex::number_complex_op(number, other, |a, b| a * b, vm)
            }),
            power: atomic_func!(|number, other, vm| PyComplex::number_general_op(
                number, other, inner_pow, vm
            )),
            negative: atomic_func!(|number, vm| {
                let value = PyComplex::number_downcast(number).value;
                (-value).to_pyresult(vm)
            }),
            positive: atomic_func!(
                |number, vm| PyComplex::number_complex(number, vm).to_pyresult(vm)
            ),
            absolute: atomic_func!(|number, vm| {
                let value = PyComplex::number_downcast(number).value;
                value.norm().to_pyresult(vm)
            }),
            boolean: atomic_func!(|number, _vm| Ok(PyComplex::number_downcast(number)
                .value
                .is_zero())),
            true_divide: atomic_func!(|number, other, vm| {
                PyComplex::number_general_op(number, other, inner_div, vm)
            }),
            ..PyNumberMethods::NOT_IMPLEMENTED
        });
        &AS_NUMBER
    }
}

impl PyComplex {
    fn number_general_op<F, R>(
        number: PyNumber,
        other: &PyObject,
        op: F,
        vm: &VirtualMachine,
    ) -> PyResult
    where
        F: FnOnce(Complex64, Complex64, &VirtualMachine) -> R,
        R: ToPyResult,
    {
        if let (Some(a), Some(b)) = (number.obj.payload::<Self>(), other.payload::<Self>()) {
            op(a.value, b.value, vm).to_pyresult(vm)
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    fn number_complex_op<F>(
        number: PyNumber,
        other: &PyObject,
        op: F,
        vm: &VirtualMachine,
    ) -> PyResult
    where
        F: FnOnce(Complex64, Complex64) -> Complex64,
    {
        Self::number_general_op(number, other, |a, b, _vm| op(a, b), vm)
    }

    fn number_complex(number: PyNumber, vm: &VirtualMachine) -> PyRef<PyComplex> {
        if let Some(zelf) = number.obj.downcast_ref_if_exact::<Self>(vm) {
            zelf.to_owned()
        } else {
            vm.ctx.new_complex(Self::number_downcast(number).value)
        }
    }
}

#[derive(FromArgs)]
pub struct ComplexArgs {
    #[pyarg(any, optional)]
    real: OptionalArg<PyObjectRef>,
    #[pyarg(any, optional)]
    imag: OptionalArg<PyObjectRef>,
}

fn parse_str(s: &str) -> Option<Complex64> {
    // Handle parentheses
    let s = match s.strip_prefix('(') {
        None => s,
        Some(s) => match s.strip_suffix(')') {
            None => return None,
            Some(s) => s.trim(),
        },
    };

    let value = match s.strip_suffix(|c| c == 'j' || c == 'J') {
        None => Complex64::new(float_ops::parse_str(s)?, 0.0),
        Some(mut s) => {
            let mut real = 0.0;
            // Find the central +/- operator. If it exists, parse the real part.
            for (i, w) in s.as_bytes().windows(2).enumerate() {
                if (w[1] == b'+' || w[1] == b'-') && !(w[0] == b'e' || w[0] == b'E') {
                    real = float_ops::parse_str(&s[..=i])?;
                    s = &s[i + 1..];
                    break;
                }
            }

            let imag = match s {
                // "j", "+j"
                "" | "+" => 1.0,
                // "-j"
                "-" => -1.0,
                s => float_ops::parse_str(s)?,
            };

            Complex64::new(real, imag)
        }
    };
    Some(value)
}
