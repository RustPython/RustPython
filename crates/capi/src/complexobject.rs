use crate::{PyObject, with_vm};
use core::ffi::c_double;
use num_complex::{Complex, Complex64};
use rustpython_vm::builtins::PyComplex;
use rustpython_vm::{PyResult, VirtualMachine};

#[unsafe(no_mangle)]
pub extern "C" fn PyComplex_FromDoubles(real: c_double, imag: c_double) -> *mut PyObject {
    with_vm(|vm| vm.ctx.new_complex(Complex::new(real, imag)))
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
        |complex| Ok(complex.to_complex()),
    )
}

#[unsafe(no_mangle)]
pub extern "C" fn PyComplex_RealAsDouble(obj: *mut PyObject) -> c_double {
    with_vm(|vm| try_to_complex(vm, unsafe { &*obj }).map(|complex| complex.re))
}

#[unsafe(no_mangle)]
pub extern "C" fn PyComplex_ImagAsDouble(obj: *mut PyObject) -> c_double {
    with_vm(|vm| try_to_complex(vm, unsafe { &*obj }).map(|complex| complex.im))
}

#[cfg(test)]
mod tests {
    use pyo3::prelude::*;
    use pyo3::types::PyComplex;

    #[test]
    fn test_py_int() {
        Python::attach(|py| {
            let number = PyComplex::from_doubles(py, 1.0, 2.0);
            assert_eq!(number.real(), 1.0);
            assert_eq!(number.imag(), 2.0);
        })
    }
}
