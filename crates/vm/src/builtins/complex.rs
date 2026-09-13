use super::{PyStr, PyType, PyTypeRef, float};
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    builtins::PyUtf8StrRef,
    class::{PyClassDef, PyClassImpl},
    common::{format::FormatSpec, wtf8::Wtf8Buf},
    convert::{IntoPyException, ToPyObject, ToPyResult},
    function::{FuncArgs, OptionalArg, PyComparisonValue},
    protocol::PyNumberMethods,
    stdlib::_warnings,
    types::{AsNumber, Callable, Comparable, Constructor, Hashable, PyComparisonOp, Representable},
};
use core::cell::Cell;
use core::num::Wrapping;
use core::ptr::NonNull;
use num_complex::Complex64;
use num_traits::Zero;
use rustpython_common::hash;

/// Create a complex number from a real part and an optional imaginary part.
///
/// This is equivalent to (real + imag*1j) where imag defaults to 0.
#[pyclass(module = false, name = "complex")]
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct PyComplex {
    value: Complex64,
}

// spell-checker:ignore MAXFREELIST
thread_local! {
    static COMPLEX_FREELIST: Cell<crate::object::FreeList<PyComplex>> = const { Cell::new(crate::object::FreeList::new()) };
}

impl PyPayload for PyComplex {
    const MAX_FREELIST: usize = 100;
    const HAS_FREELIST: bool = true;

    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.complex_type
    }

    #[inline]
    unsafe fn freelist_push(obj: *mut PyObject) -> bool {
        COMPLEX_FREELIST
            .try_with(|fl| {
                let mut list = fl.take();
                let stored = if list.len() < Self::MAX_FREELIST {
                    list.push(obj);
                    true
                } else {
                    false
                };
                fl.set(list);
                stored
            })
            .unwrap_or(false)
    }

    #[inline]
    unsafe fn freelist_pop(_payload: &Self) -> Option<NonNull<PyObject>> {
        COMPLEX_FREELIST
            .try_with(|fl| {
                let mut list = fl.take();
                let result = list.pop().map(|p| unsafe { NonNull::new_unchecked(p) });
                fl.set(list);
                result
            })
            .ok()
            .flatten()
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
                _warnings::warn(
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
            }

            return match result.downcast_ref::<PyComplex>() {
                Some(complex_obj) => Ok(Some((complex_obj.value, true))),
                None => Err(vm.new_type_error(format!(
                    "__complex__ returned non-complex (type '{}')",
                    result.class().name()
                ))),
            };
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

pub(crate) fn init(context: &'static Context) {
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

const ONE: Complex64 = Complex64::new(1.0, 0.0);

/// Only the magnitude of an infinite part matters once a product has gone to
/// nan, so it stands in as a signed one; a nan beside it stands in as a
/// signed zero.
fn signed_unit(part: f64) -> f64 {
    if part.is_infinite() { 1.0 } else { 0.0f64 }.copysign(part)
}

fn tamed(value: Complex64) -> Complex64 {
    Complex64::new(signed_unit(value.re), signed_unit(value.im))
}

fn nans_to_zero(value: Complex64) -> Complex64 {
    let zeroed = |part: f64| {
        if part.is_nan() {
            0.0f64.copysign(part)
        } else {
            part
        }
    };
    Complex64::new(zeroed(value.re), zeroed(value.im))
}

/// Multiply, recovering the infinities that the plain formula turns into nan,
/// the way C11 Annex G.5.1 does.
fn prod(a: Complex64, b: Complex64) -> Complex64 {
    let r = a * b;
    if !(r.re.is_nan() && r.im.is_nan()) {
        return r;
    }

    // "Box" an infinite operand into a signed one and turn the other's nans
    // into signed zeros, so that the infinity survives a second pass.
    let (mut a, mut b) = (a, b);
    let a_infinite = a.re.is_infinite() || a.im.is_infinite();
    if a_infinite {
        a = tamed(a);
        b = nans_to_zero(b);
    }
    let b_infinite = b.re.is_infinite() || b.im.is_infinite();
    if b_infinite {
        b = tamed(b);
        a = nans_to_zero(a);
    }
    // An infinity that overflow lost, rather than one an operand carried.
    let overflowed = !a_infinite
        && !b_infinite
        && ((a.re * b.re).is_infinite()
            || (a.im * b.im).is_infinite()
            || (a.re * b.im).is_infinite()
            || (a.im * b.re).is_infinite());
    if overflowed {
        a = nans_to_zero(a);
        b = nans_to_zero(b);
    }

    if !(a_infinite || b_infinite || overflowed) {
        return r;
    }
    Complex64::new(
        f64::INFINITY * (a.re * b.re - a.im * b.im),
        f64::INFINITY * (a.re * b.im + a.im * b.re),
    )
}

/// Divide a real by a complex. Written out rather than routed through `quot`
/// with a zero imaginary part, which would lose the sign of a zero.
fn rc_quot(a: f64, b: Complex64) -> Option<Complex64> {
    let abs_re = b.re.abs();
    let abs_im = b.im.abs();

    let r = if abs_re >= abs_im {
        if abs_re == 0.0 {
            return None;
        }
        let ratio = b.im / b.re;
        let denom = b.re + b.im * ratio;
        Complex64::new(a / denom, -a * ratio / denom)
    } else if abs_im >= abs_re {
        let ratio = b.re / b.im;
        let denom = b.re * ratio + b.im;
        Complex64::new(a * ratio / denom, -a / denom)
    } else {
        // One part of the divisor is a nan, so neither comparison held.
        Complex64::new(f64::NAN, f64::NAN)
    };

    // A quotient that came out as nan recovers the way `recovered_quot` does,
    // except that a real numerator has no imaginary term to carry a sign.
    if r.re.is_nan() && r.im.is_nan() && (b.re.is_infinite() || b.im.is_infinite()) && a.is_finite()
    {
        let Complex64 { re: x, im: y } = tamed(b);
        return Some(Complex64::new(0.0 * (a * x), 0.0 * -(a * y)));
    }

    Some(r)
}

/// Divide by whichever part of the divisor is larger, so that the ratio the
/// other part is scaled by cannot overflow. `None` stands for a divisor of
/// zero.
fn quot(a: Complex64, b: Complex64) -> Option<Complex64> {
    let abs_re = b.re.abs();
    let abs_im = b.im.abs();

    let r = if abs_re >= abs_im {
        if abs_re == 0.0 {
            return None;
        }
        let ratio = b.im / b.re;
        let denom = b.re + b.im * ratio;
        Complex64::new((a.re + a.im * ratio) / denom, (a.im - a.re * ratio) / denom)
    } else if abs_im >= abs_re {
        let ratio = b.re / b.im;
        let denom = b.re * ratio + b.im;
        Complex64::new((a.re * ratio + a.im) / denom, (a.im * ratio - a.re) / denom)
    } else {
        // One part of the divisor is a nan, so neither comparison held.
        Complex64::new(f64::NAN, f64::NAN)
    };

    Some(recovered_quot(r, a, b))
}

/// Recover the infinities and zeros a quotient came out of as nan, the way
/// C11 Annex G.5.2 does.
fn recovered_quot(r: Complex64, a: Complex64, b: Complex64) -> Complex64 {
    if !(r.re.is_nan() && r.im.is_nan()) {
        return r;
    }

    if (a.re.is_infinite() || a.im.is_infinite()) && b.re.is_finite() && b.im.is_finite() {
        let Complex64 { re: x, im: y } = tamed(a);
        Complex64::new(
            f64::INFINITY * (x * b.re + y * b.im),
            f64::INFINITY * (y * b.re - x * b.im),
        )
    } else if (b.re.is_infinite() || b.im.is_infinite()) && a.re.is_finite() && a.im.is_finite() {
        let Complex64 { re: x, im: y } = tamed(b);
        Complex64::new(0.0 * (a.re * x + a.im * y), 0.0 * (a.im * x - a.re * y))
    } else {
        r
    }
}

fn inner_div(v1: Complex64, v2: Complex64, vm: &VirtualMachine) -> PyResult<Complex64> {
    quot(v1, v2).ok_or_else(|| vm.new_zero_division_error("division by zero"))
}

/// Raise to a non-negative integer power by repeated squaring.
fn pow_unsigned(x: Complex64, n: u32) -> Complex64 {
    let mut r = ONE;
    let mut p = x;
    let mut mask = 1u32;
    while mask > 0 && n >= mask {
        if n & mask != 0 {
            r = prod(r, p);
        }
        mask <<= 1;
        p = prod(p, p);
    }
    r
}

/// Raise to an integer power. `None` stands for zero raised to a negative one.
fn powi(x: Complex64, n: i32) -> Option<Complex64> {
    if n > 0 {
        Some(pow_unsigned(x, n as u32))
    } else {
        quot(ONE, pow_unsigned(x, n.unsigned_abs()))
    }
}

pub(crate) fn complex_pow(
    v1: Complex64,
    v2: Complex64,
    vm: &VirtualMachine,
) -> PyResult<Complex64> {
    // A small integer power is reached by multiplying, which stays exact
    // where going through the polar form would not.
    let exponent = v2.re as i32;
    let result = if v2.im == 0.0 && v2.re == f64::from(exponent) && v2.re.abs() <= 100.0 {
        powi(v1, exponent)
    } else if v1.is_zero() && (v2.im != 0.0 || v2.re < 0.0) {
        None
    } else {
        Some(powc(v1, v2))
    };

    let Some(result) = result else {
        return Err(vm.new_zero_division_error("zero to a negative or complex power"));
    };
    if result.re.is_infinite() || result.im.is_infinite() {
        return Err(vm.new_overflow_error("complex exponentiation"));
    }
    Ok(result)
}

/// Raise to a power through the polar form.
fn powc(a: Complex64, exp: Complex64) -> Complex64 {
    if exp.is_zero() {
        return ONE;
    }
    if a.is_zero() {
        return Complex64::new(0.0, 0.0);
    }

    let magnitude = a.norm();
    let angle = a.arg();
    let mut len = magnitude.powf(exp.re);
    let mut phase = angle * exp.re;
    // An exponent with no imaginary part leaves these alone, and reaching for
    // them anyway would turn an infinite magnitude into a nan.
    if exp.im != 0.0 {
        len *= (-angle * exp.im).exp();
        phase += exp.im * magnitude.ln();
    }
    Complex64::new(len * phase.cos(), len * phase.sin())
}

/// Whether an underscore sits where a numeric literal does not allow one:
/// they only ever join two digits.
fn has_misplaced_underscore(bytes: &[u8]) -> bool {
    let mut prev = b'\0';
    for &byte in bytes {
        if byte == b'_' {
            if !prev.is_ascii_digit() {
                return true;
            }
        } else if prev == b'_' && !byte.is_ascii_digit() {
            return true;
        }
        prev = byte;
    }
    prev == b'_'
}

impl Constructor for PyComplex {
    type Args = ComplexArgs;

    fn slot_new(cls: PyTypeRef, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        // Optimization: return exact complex as-is (only when imag is not provided)
        if cls.is(vm.ctx.types.complex_type)
            && func_args.args.len() == 1
            && func_args.kwargs.is_empty()
            && func_args.args[0].class().is(vm.ctx.types.complex_type)
        {
            return Ok(func_args.args[0].clone());
        }

        let args: Self::Args = func_args.bind_for(vm, Self::NAME)?;
        let payload = Self::py_new(&cls, args, vm)?;
        payload.into_ref_with_type(vm, cls).map(Into::into)
    }

    fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
        let imag_missing = args.imag.is_missing();
        let (real, real_was_complex) = match args.real {
            OptionalArg::Missing => (Complex64::new(0.0, 0.0), false),
            OptionalArg::Present(val) => {
                if let Some(c) = val.try_complex(vm)? {
                    c
                } else if let Some(s) = val.downcast_ref::<PyStr>() {
                    if args.imag.is_present() {
                        return Err(vm.new_type_error(
                            "complex() can't take second arg if first is a string",
                        ));
                    }
                    if has_misplaced_underscore(s.as_wtf8().as_bytes()) {
                        let repr = val.repr(vm)?;
                        return Err(vm.new_value_error(format!(
                            "could not convert string to complex: {repr}"
                        )));
                    }
                    let (re, im) = rustpython_literal::complex::parse_str(
                        &crate::protocol::numeric_literal_from_str(s),
                    )
                    .ok_or_else(|| vm.new_value_error("complex() arg is a malformed string"))?;
                    return Ok(Self::from(Complex64 { re, im }));
                } else {
                    return Err(vm.new_type_error(format!(
                        "complex() argument must be a string or a number, not {}",
                        val.class().slot_name()
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
        Ok(Self::from(value))
    }
}

impl PyComplex {
    #[deprecated(note = "use PyComplex::from(...).into_ref() instead")]
    pub fn new_ref(value: Complex64, ctx: &Context) -> PyRef<Self> {
        Self::from(value).into_ref(ctx)
    }

    #[must_use]
    pub const fn to_complex64(self) -> Complex64 {
        self.value
    }

    #[must_use]
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

    fn complex_real_binop<CCF, RCF, CRF, R>(
        a: &PyObject,
        b: &PyObject,
        cc_op: CCF,
        cr_op: CRF,
        rc_op: RCF,
        vm: &VirtualMachine,
    ) -> PyResult
    where
        CCF: FnOnce(Complex64, Complex64) -> R,
        CRF: FnOnce(Complex64, f64) -> R,
        RCF: FnOnce(f64, Complex64) -> R,
        R: ToPyResult,
    {
        let value = match (a.downcast_ref::<Self>(), b.downcast_ref::<Self>()) {
            // complex + complex
            (Some(a_complex), Some(b_complex)) => cc_op(a_complex.value, b_complex.value),
            (Some(a_complex), None) => {
                let Some(b_real) = float::to_op_float(b, vm)? else {
                    return Ok(vm.ctx.not_implemented());
                };

                // complex + real
                cr_op(a_complex.value, b_real)
            }
            (None, Some(b_complex)) => {
                let Some(a_real) = float::to_op_float(a, vm)? else {
                    return Ok(vm.ctx.not_implemented());
                };

                // real + complex
                rc_op(a_real, b_complex.value)
            }
            (None, None) => return Ok(vm.ctx.not_implemented()),
        };
        value.to_pyresult(vm)
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
    fn conjugate(&self) -> Complex64 {
        self.value.conj()
    }

    #[pymethod]
    const fn __getnewargs__(&self) -> (f64, f64) {
        let Complex64 { re, im } = self.value;
        (re, im)
    }

    #[pymethod]
    fn __format__(zelf: &Py<Self>, spec: PyUtf8StrRef, vm: &VirtualMachine) -> PyResult<Wtf8Buf> {
        // Empty format spec: equivalent to str(self)
        if spec.is_empty() {
            return Ok(zelf.as_object().str(vm)?.as_wtf8().to_owned());
        }
        let format_spec =
            FormatSpec::parse(spec.as_str()).map_err(|err| err.into_pyexception(vm))?;
        let result = if format_spec.has_locale_format() {
            let locale = crate::format::get_locale_info();
            format_spec.format_complex_locale(&zelf.value, &locale)
        } else {
            format_spec.format_complex(&zelf.value)
        };
        result
            .map(Wtf8Buf::from_string)
            .map_err(|err| err.into_pyexception(vm))
    }

    #[pyclassmethod]
    fn from_number(cls: PyTypeRef, number: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if number.class().is(vm.ctx.types.complex_type) && cls.is(vm.ctx.types.complex_type) {
            return Ok(number);
        }
        let value = number
            .try_complex(vm)?
            .ok_or_else(|| {
                vm.new_type_error(format!(
                    "must be real number, not {}",
                    number.class().name()
                ))
            })?
            .0;
        let result = vm.ctx.new_complex(value);
        if cls.is(vm.ctx.types.complex_type) {
            Ok(result.into())
        } else {
            PyType::call(&cls, vec![result.into()].into(), vm)
        }
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
                zelf.value == other.value
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
            add: Some(|a, b, vm| {
                PyComplex::complex_real_binop(
                    a,
                    b,
                    |a, b| a + b,
                    |a_complex, b_real| Complex64::new(a_complex.re + b_real, a_complex.im),
                    |a_real, b_complex| Complex64::new(a_real + b_complex.re, b_complex.im),
                    vm,
                )
            }),
            subtract: Some(|a, b, vm| {
                PyComplex::complex_real_binop(
                    a,
                    b,
                    |a, b| a - b,
                    |a_complex, b_real| Complex64::new(a_complex.re - b_real, a_complex.im),
                    |a_real, b_complex| Complex64::new(a_real - b_complex.re, -b_complex.im),
                    vm,
                )
            }),
            multiply: Some(|a, b, vm| {
                PyComplex::complex_real_binop(
                    a,
                    b,
                    prod,
                    |a_complex, b_real| {
                        Complex64::new(a_complex.re * b_real, a_complex.im * b_real)
                    },
                    |a_real, b_complex| {
                        Complex64::new(a_real * b_complex.re, a_real * b_complex.im)
                    },
                    vm,
                )
            }),
            power: Some(|a, b, c, vm| {
                if vm.is_none(c) {
                    PyComplex::number_op(a, b, complex_pow, vm)
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
                let result = value.norm();
                // Check for overflow: hypot returns inf for finite inputs that overflow
                if result.is_infinite() && value.re.is_finite() && value.im.is_finite() {
                    return Err(vm.new_overflow_error("absolute value too large"));
                }
                result.to_pyresult(vm)
            }),
            boolean: Some(|number, _vm| Ok(!PyComplex::number_downcast(number).value.is_zero())),
            true_divide: Some(|a, b, vm| {
                PyComplex::complex_real_binop(
                    a,
                    b,
                    |a, b| inner_div(a, b, vm),
                    |a_complex, b_real| {
                        if b_real == 0.0 {
                            Err(vm.new_zero_division_error("division by zero"))
                        } else {
                            Ok(Complex64::new(a_complex.re / b_real, a_complex.im / b_real))
                        }
                    },
                    |a_real, b_complex| {
                        rc_quot(a_real, b_complex)
                            .ok_or_else(|| vm.new_zero_division_error("division by zero"))
                    },
                    vm,
                )
            }),
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
