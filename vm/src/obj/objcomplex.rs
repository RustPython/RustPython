use num_complex::Complex64;
use num_traits::{ToPrimitive, Zero};

use crate::function::OptionalArg;
use crate::pyobject::{PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

use super::objfloat::{self, PyFloat};
use super::objint;
use super::objtype::{self, PyClassRef};

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

impl From<Complex64> for PyComplex {
    fn from(value: Complex64) -> Self {
        PyComplex { value }
    }
}

pub fn init(context: &PyContext) {
    PyComplex::extend_class(context, &context.complex_type);
    let complex_doc =
        "Create a complex number from a real part and an optional imaginary part.\n\n\
         This is equivalent to (real + imag*1j) where imag defaults to 0.";

    extend_class!(context, &context.complex_type, {
        "__doc__" => context.new_str(complex_doc.to_string()),
        "__new__" => context.new_rustfunc(PyComplexRef::new),
    });
}

pub fn get_value(obj: &PyObjectRef) -> Complex64 {
    obj.payload::<PyComplex>().unwrap().value
}

impl PyComplexRef {
    fn new(
        cls: PyClassRef,
        real: OptionalArg<PyObjectRef>,
        imag: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyComplexRef> {
        let real = match real {
            OptionalArg::Missing => 0.0,
            OptionalArg::Present(ref value) => objfloat::make_float(vm, value)?,
        };

        let imag = match imag {
            OptionalArg::Missing => 0.0,
            OptionalArg::Present(ref value) => objfloat::make_float(vm, value)?,
        };

        let value = Complex64::new(real, imag);
        PyComplex { value }.into_ref_with_type(vm, cls)
    }
}

fn to_complex(value: PyObjectRef, vm: &VirtualMachine) -> PyResult<Option<Complex64>> {
    if objtype::isinstance(&value, &vm.ctx.int_type()) {
        match objint::get_value(&value).to_f64() {
            Some(v) => Ok(Some(Complex64::new(v, 0.0))),
            None => Err(vm.new_overflow_error("int too large to convert to float".to_string())),
        }
    } else if objtype::isinstance(&value, &vm.ctx.float_type()) {
        let v = objfloat::get_value(&value);
        Ok(Some(Complex64::new(v, 0.0)))
    } else {
        Ok(None)
    }
}

#[pyimpl]
impl PyComplex {
    #[pyproperty(name = "real")]
    fn real(&self, _vm: &VirtualMachine) -> PyFloat {
        self.value.re.into()
    }

    #[pyproperty(name = "imag")]
    fn imag(&self, _vm: &VirtualMachine) -> PyFloat {
        self.value.im.into()
    }

    #[pymethod(name = "__abs__")]
    fn abs(&self, _vm: &VirtualMachine) -> PyFloat {
        let Complex64 { im, re } = self.value;
        re.hypot(im).into()
    }

    #[pymethod(name = "__add__")]
    fn add(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if objtype::isinstance(&other, &vm.ctx.complex_type()) {
            Ok(vm.ctx.new_complex(self.value + get_value(&other)))
        } else {
            self.radd(other, vm)
        }
    }

    #[pymethod(name = "__radd__")]
    fn radd(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        match to_complex(other, vm) {
            Ok(Some(other)) => Ok(vm.ctx.new_complex(self.value + other)),
            Ok(None) => Ok(vm.ctx.not_implemented()),
            Err(err) => Err(err),
        }
    }

    #[pymethod(name = "__sub__")]
    fn sub(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if objtype::isinstance(&other, &vm.ctx.complex_type()) {
            Ok(vm.ctx.new_complex(self.value - get_value(&other)))
        } else {
            match to_complex(other, vm) {
                Ok(Some(other)) => Ok(vm.ctx.new_complex(self.value - other)),
                Ok(None) => Ok(vm.ctx.not_implemented()),
                Err(err) => Err(err),
            }
        }
    }

    #[pymethod(name = "__rsub__")]
    fn rsub(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        match to_complex(other, vm) {
            Ok(Some(other)) => Ok(vm.ctx.new_complex(other - self.value)),
            Ok(None) => Ok(vm.ctx.not_implemented()),
            Err(err) => Err(err),
        }
    }

    #[pymethod(name = "conjugate")]
    fn conjugate(&self, _vm: &VirtualMachine) -> PyComplex {
        self.value.conj().into()
    }

    #[pymethod(name = "__eq__")]
    fn eq(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        let result = if objtype::isinstance(&other, &vm.ctx.complex_type()) {
            self.value == get_value(&other)
        } else if objtype::isinstance(&other, &vm.ctx.int_type()) {
            match objint::get_value(&other).to_f64() {
                Some(f) => self.value.im == 0.0f64 && self.value.re == f,
                None => false,
            }
        } else if objtype::isinstance(&other, &vm.ctx.float_type()) {
            self.value.im == 0.0 && self.value.re == objfloat::get_value(&other)
        } else {
            return vm.ctx.not_implemented();
        };

        vm.ctx.new_bool(result)
    }

    #[pymethod(name = "__mul__")]
    fn mul(self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        match to_complex(other, vm) {
            Ok(Some(other)) => Ok(vm.ctx.new_complex(Complex64::new(
                self.value.re * other.re - self.value.im * other.im,
                self.value.re * other.im + self.value.re * other.im,
            ))),
            Ok(None) => Ok(vm.ctx.not_implemented()),
            Err(err) => Err(err),
        }
    }

    #[pymethod(name = "__neg__")]
    fn neg(&self, _vm: &VirtualMachine) -> PyComplex {
        PyComplex::from(-self.value)
    }

    #[pymethod(name = "__repr__")]
    fn repr(&self, _vm: &VirtualMachine) -> String {
        let Complex64 { re, im } = self.value;
        if re == 0.0 {
            format!("{}j", im)
        } else {
            format!("({}+{}j)", re, im)
        }
    }

    #[pymethod(name = "__bool__")]
    fn bool(&self, _vm: &VirtualMachine) -> bool {
        self.value != Complex64::zero()
    }
}
