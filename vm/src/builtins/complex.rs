use super::{PyStr, PyType, PyTypeRef, float};
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    builtins::PyStrRef,
    class::PyClassImpl,
    common::format::FormatSpec,
    convert::{IntoPyException, ToPyObject, ToPyResult},
    function::{
        OptionalArg, OptionalOption,
        PyArithmeticValue::{self, *},
        PyComparisonValue,
    },
    identifier,
    protocol::PyNumberMethods,
    stdlib::warnings,
    types::{AsNumber, Comparable, Constructor, Hashable, PyComparisonOp, Representable},
};
use num_complex::Complex64;
use num_traits::Zero;
use rustpython_common::hash;
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
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.complex_type
    }
}

impl ToPyObject for Complex64 {
    fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        PyComplex::from(self).to_pyobject(vm)
    }
}

impl From<Complex64> for PyComplex {
    fn from(value: Complex64) -> Self {
        Self { value }
    }
}

impl PyObjectRef {
    /// Tries converting a python object into a complex, returns an option of whether the complex
    /// and whether the  object was a complex originally or coerced into one
    pub fn try_complex(&self, vm: &VirtualMachine) -> PyResult<Option<(Complex64, bool)>> {
        if let Some(complex) = self.downcast_ref_if_exact::<PyComplex>(vm) {
            return Ok(Some((complex.value, true)));
        }
        if let Some(method) = vm.get_method(self.clone(), identifier!(vm, __complex__)) {
            let result = method?.call((), vm)?;

            let ret_class = result.class().to_owned();
            if let Some(ret) = result.downcast_ref::<PyComplex>() {
                warnings::warn(
                    vm.ctx.exceptions.deprecation_warning,
                    format!(
                        "__complex__ returned non-complex (type {ret_class}).  \
                    The ability to return an instance of a strict subclass of complex \
                    is deprecated, and may be removed in a future version of Python."
                    ),
                    1,
                    vm,
                )?;

                return Ok(Some((ret.value, true)));
            } else {
                return match result.downcast_ref::<PyComplex>() {
                    Some(complex_obj) => Ok(Some((complex_obj.value, true))),
                    None => Err(vm.new_type_error(format!(
                        "__complex__ returned non-complex (type '{}')",
                        result.class().name()
                    ))),
                };
            }
        }
        // `complex` does not have a `__complex__` by default, so subclasses might not either,
        // use the actual stored value in this case
        if let Some(complex) = self.downcast_ref::<PyComplex>() {
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
    let r = if let Some(complex) = value.downcast_ref::<PyComplex>() {
        Some(complex.value)
    } else {
        float::to_op_float(value, vm)?.map(|float| Complex64::new(float, 0.0))
    };
    Ok(r)
}

fn inner_div(v1: Complex64, v2: Complex64, vm: &VirtualMachine) -> PyResult<Complex64> {
    if v2.is_zero() {
        return Err(vm.new_zero_division_error("complex division by zero"));
    }

    Ok(v1.fdiv(v2))
}

fn inner_pow(v1: Complex64, v2: Complex64, vm: &VirtualMachine) -> PyResult<Complex64> {
    if v1.is_zero() {
        return if v2.re < 0.0 || v2.im != 0.0 {
            let msg = format!("{v1} cannot be raised to a negative or complex power");
            Err(vm.new_zero_division_error(msg))
        } else if v2.is_zero() {
            Ok(Complex64::new(1.0, 0.0))
        } else {
            Ok(Complex64::new(0.0, 0.0))
        };
    }

    let ans = powc(v1, v2);
    if ans.is_infinite() && !(v1.is_infinite() || v2.is_infinite()) {
        Err(vm.new_overflow_error("complex exponentiation overflow"))
    } else {
        Ok(ans)
    }
}

// num-complex changed their powc() implementation in 0.4.4, making it incompatible
// with what the regression tests expect. this is that old formula.
fn powc(a: Complex64, exp: Complex64) -> Complex64 {
    let (r, theta) = a.to_polar();
    if r.is_zero() {
        return Complex64::new(r, r);
    }
    Complex64::from_polar(
        r.powf(exp.re) * (-exp.im * theta).exp(),
        exp.re * theta + exp.im * r.ln(),
    )
}

impl Constructor for PyComplex {
    type Args = ComplexArgs;

    fn py_new(cls: PyTypeRef, args: Self::Args, vm: &VirtualMachine) -> PyResult {
        let imag_missing = args.imag.is_missing();
        let (real, real_was_complex) = match args.real {
            OptionalArg::Missing => (Complex64::new(0.0, 0.0), false),
            OptionalArg::Present(val) => {
                let val = if cls.is(vm.ctx.types.complex_type) && imag_missing {
                    match val.downcast_exact::<Self>(vm) {
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
                } else if let Some(s) = val.downcast_ref::<PyStr>() {
                    if args.imag.is_present() {
                        return Err(vm.new_type_error(
                            "complex() can't take second arg if first is a string",
                        ));
                    }
                    let (re, im) = s
                        .to_str()
                        .and_then(rustpython_literal::complex::parse_str)
                        .ok_or_else(|| vm.new_value_error("complex() arg is a malformed string"))?;
                    return Self::from(Complex64 { re, im })
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
                    return Err(vm.new_type_error("complex() second arg can't be a string"));
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
    #[deprecated(note = "use PyComplex::from(...).into_ref() instead")]
    pub fn new_ref(value: Complex64, ctx: &Context) -> PyRef<Self> {
        Self::from(value).into_ref(ctx)
    }

    pub const fn to_complex64(self) -> Complex64 {
        self.value
    }

    pub const fn to_complex(&self) -> Complex64 {
        self.value
    }

    fn number_op<F, R>(a: &PyObject, b: &PyObject, op: F, vm: &VirtualMachine) -> PyResult
    where
        F: FnOnce(Complex64, Complex64, &VirtualMachine) -> R,
        R: ToPyResult,
    {
        if let (Some(a), Some(b)) = (to_op_complex(a, vm)?, to_op_complex(b, vm)?) {
            op(a, b, vm).to_pyresult(vm)
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }
}

#[pyclass(
    flags(BASETYPE),
    with(PyRef, Comparable, Hashable, Constructor, AsNumber, Representable)
)]
impl PyComplex {
    #[pygetset]
    const fn real(&self) -> f64 {
        self.value.re
    }

    #[pygetset]
    const fn imag(&self) -> f64 {
        self.value.im
    }

    #[pymethod]
    fn __abs__(&self, vm: &VirtualMachine) -> PyResult<f64> {
        let Complex64 { im, re } = self.value;
        let is_finite = im.is_finite() && re.is_finite();
        let abs_result = re.hypot(im);
        if is_finite && abs_result.is_infinite() {
            Err(vm.new_overflow_error("absolute value too large"))
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
    #[pymethod]
    fn __add__(
        &self,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyArithmeticValue<Complex64>> {
        self.op(other, |a, b| Ok(a + b), vm)
    }

    #[pymethod]
    fn __sub__(
        &self,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyArithmeticValue<Complex64>> {
        self.op(other, |a, b| Ok(a - b), vm)
    }

    #[pymethod]
    fn __rsub__(
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
    #[pymethod]
    fn __mul__(
        &self,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyArithmeticValue<Complex64>> {
        self.op(other, |a, b| Ok(a * b), vm)
    }

    #[pymethod]
    fn __truediv__(
        &self,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyArithmeticValue<Complex64>> {
        self.op(other, |a, b| inner_div(a, b, vm), vm)
    }

    #[pymethod]
    fn __rtruediv__(
        &self,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyArithmeticValue<Complex64>> {
        self.op(other, |a, b| inner_div(b, a, vm), vm)
    }

    #[pymethod]
    const fn __pos__(&self) -> Complex64 {
        self.value
    }

    #[pymethod]
    fn __neg__(&self) -> Complex64 {
        -self.value
    }

    #[pymethod]
    fn __pow__(
        &self,
        other: PyObjectRef,
        mod_val: OptionalOption<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyArithmeticValue<Complex64>> {
        if mod_val.flatten().is_some() {
            Err(vm.new_value_error("complex modulo not allowed"))
        } else {
            self.op(other, |a, b| inner_pow(a, b, vm), vm)
        }
    }

    #[pymethod]
    fn __rpow__(
        &self,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyArithmeticValue<Complex64>> {
        self.op(other, |a, b| inner_pow(b, a, vm), vm)
    }

    #[pymethod]
    fn __bool__(&self) -> bool {
        !Complex64::is_zero(&self.value)
    }

    #[pymethod]
    const fn __getnewargs__(&self) -> (f64, f64) {
        let Complex64 { re, im } = self.value;
        (re, im)
    }

    #[pymethod]
    fn __format__(&self, spec: PyStrRef, vm: &VirtualMachine) -> PyResult<String> {
        FormatSpec::parse(spec.as_str())
            .and_then(|format_spec| format_spec.format_complex(&self.value))
            .map_err(|err| err.into_pyexception(vm))
    }
}

#[pyclass]
impl PyRef<PyComplex> {
    #[pymethod]
    fn __complex__(self, vm: &VirtualMachine) -> Self {
        if self.is(vm.ctx.types.complex_type) {
            self
        } else {
            PyComplex::from(self.value).into_ref(&vm.ctx)
        }
    }
}

impl Comparable for PyComplex {
    fn cmp(
        zelf: &Py<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        op.eq_only(|| {
            let result = if let Some(other) = other.downcast_ref::<Self>() {
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
    fn hash(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<hash::PyHash> {
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
        static AS_NUMBER: PyNumberMethods = PyNumberMethods {
            add: Some(|a, b, vm| PyComplex::number_op(a, b, |a, b, _vm| a + b, vm)),
            subtract: Some(|a, b, vm| PyComplex::number_op(a, b, |a, b, _vm| a - b, vm)),
            multiply: Some(|a, b, vm| PyComplex::number_op(a, b, |a, b, _vm| a * b, vm)),
            power: Some(|a, b, c, vm| {
                if vm.is_none(c) {
                    PyComplex::number_op(a, b, inner_pow, vm)
                } else {
                    Err(vm.new_value_error(String::from("complex modulo")))
                }
            }),
            negative: Some(|number, vm| {
                let value = PyComplex::number_downcast(number).value;
                (-value).to_pyresult(vm)
            }),
            positive: Some(|number, vm| {
                PyComplex::number_downcast_exact(number, vm).to_pyresult(vm)
            }),
            absolute: Some(|number, vm| {
                let value = PyComplex::number_downcast(number).value;
                value.norm().to_pyresult(vm)
            }),
            boolean: Some(|number, _vm| Ok(PyComplex::number_downcast(number).value.is_zero())),
            true_divide: Some(|a, b, vm| PyComplex::number_op(a, b, inner_div, vm)),
            ..PyNumberMethods::NOT_IMPLEMENTED
        };
        &AS_NUMBER
    }

    fn clone_exact(zelf: &Py<Self>, vm: &VirtualMachine) -> PyRef<Self> {
        vm.ctx.new_complex(zelf.value)
    }
}

impl Representable for PyComplex {
    #[inline]
    fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
        // TODO: when you fix this, move it to rustpython_common::complex::repr and update
        //       ast/src/unparse.rs + impl Display for Constant in ast/src/constant.rs
        let Complex64 { re, im } = zelf.value;
        Ok(rustpython_literal::complex::to_string(re, im))
    }
}

#[derive(FromArgs)]
pub struct ComplexArgs {
    #[pyarg(any, optional)]
    real: OptionalArg<PyObjectRef>,
    #[pyarg(any, optional)]
    imag: OptionalArg<PyObjectRef>,
}
