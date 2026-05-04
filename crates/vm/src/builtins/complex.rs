use super::{PyStr, PyType, PyTypeRef, float};
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    builtins::PyUtf8StrRef,
    class::PyClassImpl,
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

fn inner_div(v1: Complex64, v2: Complex64, vm: &VirtualMachine) -> PyResult<Complex64> {
    if v2.is_zero() {
        return Err(vm.new_zero_division_error("complex division by zero"));
    }

    Ok(v1.fdiv(v2))
}

pub(crate) fn complex_pow(
    v1: Complex64,
    v2: Complex64,
    vm: &VirtualMachine,
) -> PyResult<Complex64> {
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
        Err(vm.new_overflow_error("complex exponentiation"))
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

    fn slot_new(cls: PyTypeRef, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        // Optimization: return exact complex as-is (only when imag is not provided)
        if cls.is(vm.ctx.types.complex_type)
            && func_args.args.len() == 1
            && func_args.kwargs.is_empty()
            && func_args.args[0].class().is(vm.ctx.types.complex_type)
        {
            return Ok(func_args.args[0].clone());
        }

        let args: Self::Args = func_args.bind(vm)?;
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
                    let (re, im) = s
                        .to_str()
                        .and_then(rustpython_literal::complex::parse_str)
                        .ok_or_else(|| vm.new_value_error("complex() arg is a malformed string"))?;
                    return Ok(Self::from(Complex64 { re, im }));
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
        Ok(Self::from(value))
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
        let value = match (a.downcast_ref::<PyComplex>(), b.downcast_ref::<PyComplex>()) {
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
            multiply: Some(|a, b, vm| PyComplex::number_op(a, b, |a, b, _vm| a * b, vm)),
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
