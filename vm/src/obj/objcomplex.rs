use num_complex::Complex64;
use num_traits::ToPrimitive;

use crate::function::{OptionalArg, PyFuncArgs};
use crate::pyobject::{PyContext, PyObjectRef, PyRef, PyResult, PyValue, TypeProtocol};
use crate::vm::VirtualMachine;

use super::objfloat;
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

    fn real(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
        arg_check!(vm, args, required = [(zelf, Some(vm.ctx.complex_type()))]);
        let Complex64 { re, .. } = get_value(zelf);
        Ok(vm.ctx.new_float(re))
    }

    fn imag(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
        arg_check!(vm, args, required = [(zelf, Some(vm.ctx.complex_type()))]);
        let Complex64 { im, .. } = get_value(zelf);
        Ok(vm.ctx.new_float(im))
    }

    fn abs(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
        arg_check!(vm, args, required = [(zelf, Some(vm.ctx.complex_type()))]);

        let Complex64 { re, im } = get_value(zelf);
        Ok(vm.ctx.new_float(re.hypot(im)))
    }

    fn add(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
        arg_check!(
            vm,
            args,
            required = [(i, Some(vm.ctx.complex_type())), (i2, None)]
        );

        let v1 = get_value(i);
        if objtype::isinstance(i2, &vm.ctx.complex_type()) {
            Ok(vm.ctx.new_complex(v1 + get_value(i2)))
        } else if objtype::isinstance(i2, &vm.ctx.int_type()) {
            Ok(vm.ctx.new_complex(Complex64::new(
                v1.re + objint::get_value(i2).to_f64().unwrap(),
                v1.im,
            )))
        } else {
            Err(vm.new_type_error(format!("Cannot add {} and {}", i, i2)))
        }
    }

    fn radd(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
        arg_check!(
            vm,
            args,
            required = [(i, Some(vm.ctx.complex_type())), (i2, None)]
        );

        let v1 = get_value(i);

        if objtype::isinstance(i2, &vm.ctx.int_type()) {
            Ok(vm.ctx.new_complex(Complex64::new(
                v1.re + objint::get_value(i2).to_f64().unwrap(),
                v1.im,
            )))
        } else {
            Err(vm.new_type_error(format!("Cannot add {} and {}", i, i2)))
        }
    }

    fn conjugate(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
        arg_check!(vm, args, required = [(i, Some(vm.ctx.complex_type()))]);

        let v1 = get_value(i);
        Ok(vm.ctx.new_complex(v1.conj()))
    }

    fn eq(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
        arg_check!(
            vm,
            args,
            required = [(zelf, Some(vm.ctx.complex_type())), (other, None)]
        );

        let z = get_value(zelf);

        let result = if objtype::isinstance(other, &vm.ctx.complex_type()) {
            z == get_value(other)
        } else if objtype::isinstance(other, &vm.ctx.int_type()) {
            match objint::get_value(other).to_f64() {
                Some(f) => z.im == 0.0f64 && z.re == f,
                None => false,
            }
        } else if objtype::isinstance(other, &vm.ctx.float_type()) {
            z.im == 0.0 && z.re == objfloat::get_value(other)
        } else {
            return Ok(vm.ctx.not_implemented());
        };

        Ok(vm.ctx.new_bool(result))
    }

    fn neg(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
        arg_check!(vm, args, required = [(zelf, Some(vm.ctx.complex_type()))]);
        Ok(vm.ctx.new_complex(-get_value(zelf)))
    }

    fn repr(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
        arg_check!(vm, args, required = [(obj, Some(vm.ctx.complex_type()))]);
        let v = get_value(obj);
        let repr = if v.re == 0. {
            format!("{}j", v.im)
        } else {
            format!("({}+{}j)", v.re, v.im)
        };
        Ok(vm.new_str(repr))
    }
}
