use num_complex::Complex64;
use num_traits::Zero;

use super::float;
use super::pystr::PyStr;
use super::pytype::PyTypeRef;
use crate::pyobject::{
    BorrowValue, IntoPyObject, Never, PyArithmaticValue, PyClassImpl, PyComparisonValue, PyContext,
    PyObjectRef, PyRef, PyResult, PyValue, TypeProtocol,
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

fn try_complex(value: &PyObjectRef, vm: &VirtualMachine) -> PyResult<Option<Complex64>> {
    let r = if let Some(complex) = value.payload_if_subclass::<PyComplex>(vm) {
        Some(complex.value)
    } else if let Some(float) = float::try_float(value, vm)? {
        Some(Complex64::new(float, 0.0))
    } else {
        None
    };
    Ok(r)
}

#[pyimpl(flags(BASETYPE), with(Comparable, Hashable))]
impl PyComplex {
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
        F: Fn(Complex64, Complex64) -> Complex64,
    {
        Ok(try_complex(&other, vm)?.map_or_else(
            || PyArithmaticValue::NotImplemented,
            |other| PyArithmaticValue::Implemented(op(self.value, other)),
        ))
    }

    #[pymethod(name = "__add__")]
    #[pymethod(name = "__radd__")]
    fn add(
        &self,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyArithmaticValue<Complex64>> {
        self.op(other, |a, b| a + b, vm)
    }

    #[pymethod(name = "__sub__")]
    fn sub(
        &self,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyArithmaticValue<Complex64>> {
        self.op(other, |a, b| a - b, vm)
    }

    #[pymethod(name = "__rsub__")]
    fn rsub(
        &self,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyArithmaticValue<Complex64>> {
        self.op(other, |a, b| b - a, vm)
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
        self.op(other, |a, b| a * b, vm)
    }

    #[pymethod(name = "__truediv__")]
    fn truediv(
        &self,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyArithmaticValue<Complex64>> {
        self.op(other, |a, b| a / b, vm)
    }

    #[pymethod(name = "__rtruediv__")]
    fn rtruediv(
        &self,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyArithmaticValue<Complex64>> {
        self.op(other, |a, b| b / a, vm)
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
        vm: &VirtualMachine,
    ) -> PyResult<PyArithmaticValue<Complex64>> {
        self.op(other, |a, b| a.powc(b), vm)
    }

    #[pymethod(name = "__rpow__")]
    fn rpow(
        &self,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyArithmaticValue<Complex64>> {
        self.op(other, |a, b| b.powc(a), vm)
    }

    #[pymethod(name = "__bool__")]
    fn bool(&self) -> bool {
        !Complex64::is_zero(&self.value)
    }

    #[pyslot]
    fn tp_new(cls: PyTypeRef, args: ComplexArgs, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        let real = match args.real {
            None => Complex64::new(0.0, 0.0),
            Some(obj) => {
                if let Some(c) = try_complex(&obj, vm)? {
                    c
                } else if let Some(s) = obj.payload_if_subclass::<PyStr>(vm) {
                    if args.imag.is_some() {
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
                        obj.class().name
                    )));
                }
            }
        };

        let imag = match args.imag {
            None => Complex64::new(0.0, 0.0),
            Some(obj) => {
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

        let value = Complex64::new(real.re - imag.im, real.im + imag.re);
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
                match float::try_float(&other, vm) {
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
    #[pyarg(any, default)]
    real: Option<PyObjectRef>,
    #[pyarg(any, default)]
    imag: Option<PyObjectRef>,
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
