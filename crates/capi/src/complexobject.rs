use crate::object::define_py_check;
use crate::util::FfiPtrExt;
use crate::{PyObject, pystate::with_vm};
use core::ffi::c_double;
use num_complex::{Complex, Complex64};
use rustpython_vm::builtins::PyComplex;
use rustpython_vm::{PyResult, VirtualMachine};

define_py_check!(fn PyComplex_Check, types.complex_type);
define_py_check!(exact fn PyComplex_CheckExact, types.complex_type);

#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct Py_complex {
    pub real: c_double,
    pub imag: c_double,
}

#[unsafe(no_mangle)]
pub extern "C" fn PyComplex_FromDoubles(real: c_double, imag: c_double) -> *mut PyObject {
    with_vm(|vm| vm.ctx.new_complex(Complex::new(real, imag)))
}

#[unsafe(no_mangle)]
pub extern "C" fn PyComplex_FromCComplex(v: Py_complex) -> *mut PyObject {
    with_vm(|vm| vm.ctx.new_complex(Complex::new(v.real, v.imag)))
}

fn try_to_complex(vm: &VirtualMachine, obj: &PyObject) -> PyResult<Complex64> {
    obj.try_downcast_ref::<PyComplex>(vm).map_or_else(
        |type_err| {
            if let Some((complex, _)) = obj.to_owned().try_complex(vm)? {
                Ok(complex)
            } else {
                Err(type_err)
            }
        },
        |complex| Ok(complex.as_complex()),
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyComplex_RealAsDouble(obj: *mut PyObject) -> c_double {
    with_vm(|vm| try_to_complex(vm, unsafe { obj.assume_borrowed() }).map(|complex| complex.re))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyComplex_ImagAsDouble(obj: *mut PyObject) -> c_double {
    with_vm(|vm| try_to_complex(vm, unsafe { obj.assume_borrowed() }).map(|complex| complex.im))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyComplex_AsCComplex(obj: *mut PyObject) -> Py_complex {
    with_vm(|vm| {
        try_to_complex(vm, unsafe { obj.assume_borrowed() }).map(|complex| Py_complex {
            real: complex.re,
            imag: complex.im,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::Py_complex;
    use pyo3::prelude::*;
    use pyo3::types::PyComplex;

    #[test]
    fn py_int() {
        Python::attach(|py| {
            let number = PyComplex::from_doubles(py, 1.0, 2.0);
            assert_eq!(number.real(), 1.0);
            assert_eq!(number.imag(), 2.0);
        })
    }

    #[test]
    fn c_complex_roundtrip() {
        Python::attach(|py| {
            let c = Py_complex {
                real: 3.5,
                imag: -4.5,
            };
            let obj_ptr = super::PyComplex_FromCComplex(c);
            let obj = unsafe { pyo3::Bound::from_owned_ptr(py, obj_ptr.cast()) };
            let roundtrip = unsafe { super::PyComplex_AsCComplex(obj.as_ptr().cast()) };
            assert_eq!(roundtrip.real, 3.5);
            assert_eq!(roundtrip.imag, -4.5);
        });
    }
}
