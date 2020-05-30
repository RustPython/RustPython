use num_complex::Complex64;
use num_traits::Zero;
use std::num::Wrapping;
use std::str::FromStr;

use super::objfloat::{self, IntoPyFloat, PyFloat};
use super::objint::{self, PyInt};
use super::objstr::PyString;
use super::objtype::PyClassRef;
use crate::function::OptionalArg;
use crate::pyhash;
use crate::pyobject::{
    IntoPyObject, PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue, TypeProtocol,
};
use crate::vm::VirtualMachine;

/// Create a complex number from a real part and an optional imaginary part.
///
/// This is equivalent to (real + imag*1j) where imag defaults to 0.
#[pyclass(name = "complex")]
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct PyComplex {
    value: Complex64,
}

type PyComplexRef = PyRef<PyComplex>;

impl PyValue for PyComplex {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.complex_type()
    }
}

impl IntoPyObject for Complex64 {
    fn into_pyobject(self, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.new_complex(self))
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
    } else if let Some(float) = objfloat::try_float(value, vm)? {
        Some(Complex64::new(float, 0.0))
    } else {
        None
    };
    Ok(r)
}

#[pyimpl(flags(BASETYPE))]
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
    fn op<F>(&self, other: PyObjectRef, op: F, vm: &VirtualMachine) -> PyResult
    where
        F: Fn(Complex64, Complex64) -> Complex64,
    {
        try_complex(&other, vm)?.map_or_else(
            || Ok(vm.ctx.not_implemented()),
            |other| op(self.value, other).into_pyobject(vm),
        )
    }

    #[pymethod(name = "__add__")]
    fn add(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.op(other, |a, b| a + b, vm)
    }

    #[pymethod(name = "__radd__")]
    fn radd(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.add(other, vm)
    }

    #[pymethod(name = "__sub__")]
    fn sub(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.op(other, |a, b| a - b, vm)
    }

    #[pymethod(name = "__rsub__")]
    fn rsub(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.op(other, |a, b| b - a, vm)
    }

    #[pymethod(name = "conjugate")]
    fn conjugate(&self) -> Complex64 {
        self.value.conj()
    }

    #[pymethod(name = "__eq__")]
    fn eq(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        let result = if let Some(other) = other.payload_if_subclass::<PyComplex>(vm) {
            self.value == other.value
        } else {
            match objfloat::try_float(&other, vm) {
                Ok(Some(other)) => self.value.im == 0.0f64 && self.value.re == other,
                Err(_) => false,
                Ok(None) => return vm.ctx.not_implemented(),
            }
        };

        vm.ctx.new_bool(result)
    }

    #[pymethod(name = "__float__")]
    fn float(&self, vm: &VirtualMachine) -> PyResult<()> {
        Err(vm.new_type_error(String::from("Can't convert complex to float")))
    }

    #[pymethod(name = "__int__")]
    fn int(&self, vm: &VirtualMachine) -> PyResult<()> {
        Err(vm.new_type_error(String::from("Can't convert complex to int")))
    }

    #[pymethod(name = "__mul__")]
    fn mul(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.op(other, |a, b| a * b, vm)
    }

    #[pymethod(name = "__rmul__")]
    fn rmul(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.mul(other, vm)
    }

    #[pymethod(name = "__truediv__")]
    fn truediv(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.op(other, |a, b| a / b, vm)
    }

    #[pymethod(name = "__rtruediv__")]
    fn rtruediv(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.op(other, |a, b| b / a, vm)
    }

    #[pymethod(name = "__mod__")]
    fn mod_(&self, _other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_type_error("can't mod complex numbers.".to_owned()))
    }

    #[pymethod(name = "__rmod__")]
    fn rmod(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.mod_(other, vm)
    }

    #[pymethod(name = "__floordiv__")]
    fn floordiv(&self, _other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_type_error("can't take floor of complex number.".to_owned()))
    }

    #[pymethod(name = "__rfloordiv__")]
    fn rfloordiv(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.floordiv(other, vm)
    }

    #[pymethod(name = "__divmod__")]
    fn divmod(&self, _other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_type_error("can't take floor or mod of complex number.".to_owned()))
    }

    #[pymethod(name = "__rdivmod__")]
    fn rdivmod(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.divmod(other, vm)
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
    fn pow(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.op(other, |a, b| a.powc(b), vm)
    }

    #[pymethod(name = "__rpow__")]
    fn rpow(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.op(other, |a, b| b.powc(a), vm)
    }

    #[pymethod(name = "__bool__")]
    fn bool(&self) -> bool {
        !Complex64::is_zero(&self.value)
    }

    #[pyslot]
    fn tp_new(
        cls: PyClassRef,
        real: OptionalArg<PyObjectRef>,
        imag: OptionalArg<IntoPyFloat>,
        vm: &VirtualMachine,
    ) -> PyResult<PyComplexRef> {
        let real = match real {
            OptionalArg::Missing => 0.0,
            OptionalArg::Present(obj) => match_class!(match obj {
                i @ PyInt => {
                    objint::try_float(i.as_bigint(), vm)?
                }
                f @ PyFloat => {
                    f.to_f64()
                }
                s @ PyString => {
                    if imag.into_option().is_some() {
                        return Err(vm.new_type_error(
                            "complex() can't take second arg if first is a string".to_owned(),
                        ));
                    }
                    let value = Complex64::from_str(s.as_str())
                        .map_err(|err| vm.new_value_error(err.to_string()))?;
                    return PyComplex { value }.into_ref_with_type(vm, cls);
                }
                obj => {
                    return Err(vm.new_type_error(format!(
                        "complex() first argument must be a string or a number, not '{}'",
                        obj.class()
                    )));
                }
            }),
        };

        let imag = match imag {
            OptionalArg::Missing => 0.0,
            OptionalArg::Present(ref value) => value.to_f64(),
        };

        let value = Complex64::new(real, imag);
        PyComplex { value }.into_ref_with_type(vm, cls)
    }

    #[pymethod(name = "__hash__")]
    fn hash(&self) -> pyhash::PyHash {
        let re_hash = pyhash::hash_float(self.value.re);
        let im_hash = pyhash::hash_float(self.value.im);
        let ret = Wrapping(re_hash) + Wrapping(im_hash) * Wrapping(pyhash::IMAG);
        ret.0
    }

    #[pymethod(name = "__getnewargs__")]
    fn complex_getnewargs(&self, vm: &VirtualMachine) -> PyObjectRef {
        let Complex64 { re, im } = self.value;
        vm.ctx
            .new_tuple(vec![vm.ctx.new_float(re), vm.ctx.new_float(im)])
    }
}
