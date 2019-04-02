use num_complex::Complex64;
use num_traits::ToPrimitive;

use crate::function::OptionalArg;
use crate::pyobject::{PyContext, PyObjectRef, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

use super::objfloat::{self, PyFloat};
use super::objint;
use super::objtype::{self, PyClassRef};

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
    let complex_type = &context.complex_type;

    let complex_doc =
        "Create a complex number from a real part and an optional imaginary part.\n\n\
         This is equivalent to (real + imag*1j) where imag defaults to 0.";

    extend_class!(context, complex_type, {
        "__doc__" => context.new_str(complex_doc.to_string()),
        "__abs__" => context.new_rustfunc(PyComplexRef::abs),
        "__add__" => context.new_rustfunc(PyComplexRef::add),
        "__eq__" => context.new_rustfunc(PyComplexRef::eq),
        "__neg__" => context.new_rustfunc(PyComplexRef::neg),
        "__new__" => context.new_rustfunc(PyComplexRef::new),
        "__radd__" => context.new_rustfunc(PyComplexRef::radd),
        "__repr__" => context.new_rustfunc(PyComplexRef::repr),
        "conjugate" => context.new_rustfunc(PyComplexRef::conjugate),
        "imag" => context.new_property(PyComplexRef::imag),
        "real" => context.new_property(PyComplexRef::real)
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

    fn real(self, _vm: &VirtualMachine) -> PyFloat {
        self.value.re.into()
    }

    fn imag(self, _vm: &VirtualMachine) -> PyFloat {
        self.value.im.into()
    }

    fn abs(self, _vm: &VirtualMachine) -> PyFloat {
        let Complex64 { im, re } = self.value;
        re.hypot(im).into()
    }

    fn add(self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if objtype::isinstance(&other, &vm.ctx.complex_type()) {
            vm.ctx.new_complex(self.value + get_value(&other))
        } else if objtype::isinstance(&other, &vm.ctx.int_type()) {
            vm.ctx.new_complex(Complex64::new(
                self.value.re + objint::get_value(&other).to_f64().unwrap(),
                self.value.im,
            ))
        } else {
            vm.ctx.not_implemented()
        }
    }

    fn radd(self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if objtype::isinstance(&other, &vm.ctx.int_type()) {
            vm.ctx.new_complex(Complex64::new(
                self.value.re + objint::get_value(&other).to_f64().unwrap(),
                self.value.im,
            ))
        } else {
            vm.ctx.not_implemented()
        }
    }

    fn conjugate(self, _vm: &VirtualMachine) -> PyComplex {
        self.value.conj().into()
    }

    fn eq(self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
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

    fn neg(self, _vm: &VirtualMachine) -> PyComplex {
        PyComplex::from(-self.value)
    }

    fn repr(self, _vm: &VirtualMachine) -> String {
        let Complex64 { re, im } = self.value;
        if re == 0.0 {
            format!("{}j", im)
        } else {
            format!("({}+{}j)", re, im)
        }
    }
}
