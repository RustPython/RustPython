use num_complex::Complex64;
use num_traits::Zero;

use super::float;
use super::pystr::PyStr;
use super::pytype::PyTypeRef;
use crate::function::{OptionalArg, OptionalOption};
use crate::pyobject::{
    BorrowValue, IdProtocol, IntoPyObject, Never,
    PyArithmaticValue::{self, *},
    PyClassImpl, PyComparisonValue, PyContext, PyObjectRef, PyRef, PyResult, PyValue, TypeProtocol,
};
use crate::slots::{Comparable, Hashable, PyComparisonOp};
use crate::VirtualMachine;
use rustpython_common::{float_ops, hash};

/// Create a complex number from a real part and an optional imaginary part.
///
/// This is equivalent to (real + imag*1j) where imag defaults to 0.
#[pyclass(module = false, name = "complex")]
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct PyComplex {
    value: Complex64,
}

impl PyValue for PyComplex {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.complex_type
    }
}

impl IntoPyObject for Complex64 {
    fn into_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_complex(self)
    }
}

impl From<Complex64> for PyComplex {
    fn from(value: Complex64) -> Self {
        PyComplex { value }
    }
}

pub fn init(context: &PyContext) {
    PyComplex::extend_class(context, &context.types.complex_type);
}

fn to_op_complex(value: &PyObjectRef, vm: &VirtualMachine) -> PyResult<Option<Complex64>> {
    let r = if let Some(complex) = value.payload_if_subclass::<PyComplex>(vm) {
        Some(complex.value)
    } else if let Some(float) = float::to_op_float(value, vm)? {
        Some(Complex64::new(float, 0.0))
    } else {
        None
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
            let msg = format!("{} cannot be raised to a negative or complex power", v1);
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

#[pyimpl(flags(BASETYPE), with(Comparable, Hashable))]
impl PyComplex {
    pub fn to_complex(&self) -> Complex64 {
        self.value
    }

    #[pyproperty(name = "real")]
    fn real(&self) -> f64 {
        self.value.re
    }

    #[pyproperty(name = "imag")]
    fn imag(&self) -> f64 {
        self.value.im
    }

    #[pymethod(name = "__abs__")]
    fn abs(&self) -> f64 {
        let Complex64 { im, re } = self.value;
        re.hypot(im)
    }

    #[inline]
    fn op<F>(
        &self,
        other: PyObjectRef,
        op: F,
        vm: &VirtualMachine,
    ) -> PyResult<PyArithmaticValue<Complex64>>
    where
        F: Fn(Complex64, Complex64) -> PyResult<Complex64>,
    {
        to_op_complex(&other, vm)?.map_or_else(
            || Ok(NotImplemented),
            |other| Ok(Implemented(op(self.value, other)?)),
        )
    }

    #[pymethod(name = "__add__")]
    #[pymethod(name = "__radd__")]
    fn add(
        &self,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyArithmaticValue<Complex64>> {
        self.op(other, |a, b| Ok(a + b), vm)
    }

    #[pymethod(name = "__sub__")]
    fn sub(
        &self,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyArithmaticValue<Complex64>> {
        self.op(other, |a, b| Ok(a - b), vm)
    }

    #[pymethod(name = "__rsub__")]
    fn rsub(
        &self,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyArithmaticValue<Complex64>> {
        self.op(other, |a, b| Ok(b - a), vm)
    }

    #[pymethod(name = "conjugate")]
    fn conjugate(&self) -> Complex64 {
        self.value.conj()
    }

    #[pymethod(name = "__float__")]
    fn float(&self, vm: &VirtualMachine) -> PyResult<Never> {
        Err(vm.new_type_error(String::from("Can't convert complex to float")))
    }

    #[pymethod(name = "__int__")]
    fn int(&self, vm: &VirtualMachine) -> PyResult<Never> {
        Err(vm.new_type_error(String::from("Can't convert complex to int")))
    }

    #[pymethod(name = "__mul__")]
    #[pymethod(name = "__rmul__")]
    fn mul(
        &self,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyArithmaticValue<Complex64>> {
        self.op(other, |a, b| Ok(a * b), vm)
    }

    #[pymethod(name = "__truediv__")]
    fn truediv(
        &self,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyArithmaticValue<Complex64>> {
        self.op(other, |a, b| inner_div(a, b, vm), vm)
    }

    #[pymethod(name = "__rtruediv__")]
    fn rtruediv(
        &self,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyArithmaticValue<Complex64>> {
        self.op(other, |a, b| inner_div(b, a, vm), vm)
    }

    #[pymethod(name = "__mod__")]
    #[pymethod(name = "__rmod__")]
    fn mod_(&self, _other: PyObjectRef, vm: &VirtualMachine) -> PyResult<Never> {
        Err(vm.new_type_error("can't mod complex numbers.".to_owned()))
    }

    #[pymethod(name = "__floordiv__")]
    #[pymethod(name = "__rfloordiv__")]
    fn floordiv(&self, _other: PyObjectRef, vm: &VirtualMachine) -> PyResult<Never> {
        Err(vm.new_type_error("can't take floor of complex number.".to_owned()))
    }

    #[pymethod(name = "__divmod__")]
    #[pymethod(name = "__rdivmod__")]
    fn divmod(&self, _other: PyObjectRef, vm: &VirtualMachine) -> PyResult<Never> {
        Err(vm.new_type_error("can't take floor or mod of complex number.".to_owned()))
    }

    #[pymethod(name = "__pos__")]
    fn pos(&self) -> Complex64 {
        self.value
    }

    #[pymethod(name = "__neg__")]
    fn neg(&self) -> Complex64 {
        -self.value
    }

    #[pymethod(name = "__repr__")]
    fn repr(&self) -> String {
        let Complex64 { re, im } = self.value;
        if re == 0.0 {
            format!("{}j", im)
        } else {
            format!("({}{:+}j)", re, im)
        }
    }

    #[pymethod(name = "__pow__")]
    fn pow(
        &self,
        other: PyObjectRef,
        mod_val: OptionalOption<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyArithmaticValue<Complex64>> {
        if mod_val.flatten().is_some() {
            Err(vm.new_value_error("complex modulo not allowed".to_owned()))
        } else {
            self.op(other, |a, b| inner_pow(a, b, vm), vm)
        }
    }

    #[pymethod(name = "__rpow__")]
    fn rpow(
        &self,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyArithmaticValue<Complex64>> {
        self.op(other, |a, b| inner_pow(b, a, vm), vm)
    }

    #[pymethod(name = "__bool__")]
    fn bool(&self) -> bool {
        !Complex64::is_zero(&self.value)
    }

    #[pyslot]
    fn tp_new(cls: PyTypeRef, args: ComplexArgs, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        let imag_missing = args.imag.is_missing();
        let (real, real_was_complex) = match args.real {
            OptionalArg::Missing => (Complex64::new(0.0, 0.0), false),
            OptionalArg::Present(val) => {
                let val = if cls.is(&vm.ctx.types.complex_type) && imag_missing {
                    match val.downcast_exact::<PyComplex>(vm) {
                        Ok(c) => {
                            return Ok(c);
                        }
                        Err(val) => val,
                    }
                } else {
                    val
                };

                if let Some(c) = try_complex(&val, vm)? {
                    c
                } else if let Some(s) = val.payload_if_subclass::<PyStr>(vm) {
                    if args.imag.is_present() {
                        return Err(vm.new_type_error(
                            "complex() can't take second arg if first is a string".to_owned(),
                        ));
                    }
                    let value = parse_str(s.borrow_value().trim()).ok_or_else(|| {
                        vm.new_value_error("complex() arg is a malformed string".to_owned())
                    })?;
                    return Self::from(value).into_ref_with_type(vm, cls);
                } else {
                    return Err(vm.new_type_error(format!(
                        "complex() first argument must be a string or a number, not '{}'",
                        val.class().name
                    )));
                }
            }
        };

        let (imag, imag_was_complex) = match args.imag {
            // Copy the imaginary from the real to the real of the imaginary
            // if an  imaginary argument is not passed in
            OptionalArg::Missing => (Complex64::new(real.im, 0.0), false),
            OptionalArg::Present(obj) => {
                if let Some(c) = try_complex(&obj, vm)? {
                    c
                } else if obj.class().issubclass(&vm.ctx.types.str_type) {
                    return Err(
                        vm.new_type_error("complex() second arg can't be a string".to_owned())
                    );
                } else {
                    return Err(vm.new_type_error(format!(
                        "complex() second argument must be a number, not '{}'",
                        obj.class().name
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
        Self::from(value).into_ref_with_type(vm, cls)
    }

    #[pymethod(name = "__getnewargs__")]
    fn complex_getnewargs(&self, vm: &VirtualMachine) -> PyObjectRef {
        let Complex64 { re, im } = self.value;
        vm.ctx
            .new_tuple(vec![vm.ctx.new_float(re), vm.ctx.new_float(im)])
    }
}

impl Comparable for PyComplex {
    fn cmp(
        zelf: &PyRef<Self>,
        other: &PyObjectRef,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        op.eq_only(|| {
            let result = if let Some(other) = other.payload_if_subclass::<PyComplex>(vm) {
                zelf.value == other.value
            } else {
                match float::to_op_float(&other, vm) {
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
    fn hash(zelf: &PyRef<Self>, _vm: &VirtualMachine) -> PyResult<hash::PyHash> {
        Ok(hash::hash_complex(&zelf.value))
    }
}

#[derive(FromArgs)]
struct ComplexArgs {
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

/// Tries converting a python object into a complex, returns an option of whether the complex
/// and whether the  object was a complex originally or coereced into one
fn try_complex(obj: &PyObjectRef, vm: &VirtualMachine) -> PyResult<Option<(Complex64, bool)>> {
    if let Some(complex) = obj.payload_if_exact::<PyComplex>(vm) {
        return Ok(Some((complex.value, true)));
    }
    if let Some(method) = vm.get_method(obj.clone(), "__complex__") {
        let result = vm.invoke(&method?, ())?;
        // TODO: returning strict subclasses of complex in __complex__ is deprecated
        return match result.payload::<PyComplex>() {
            Some(complex_obj) => Ok(Some((complex_obj.value, true))),
            None => Err(vm.new_type_error(format!(
                "__complex__ returned non-complex (type '{}')",
                result.class().name
            ))),
        };
    }
    // `complex` does not have a `__complex__` by default, so subclasses might not either,
    // use the actual stored value in this case
    if let Some(complex) = obj.payload_if_subclass::<PyComplex>(vm) {
        return Ok(Some((complex.value, true)));
    }
    if let Some(float) = float::try_float_opt(obj, vm)? {
        return Ok(Some((Complex64::new(float, 0.0), false)));
    }
    Ok(None)
}
