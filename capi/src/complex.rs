// https://docs.python.org/3/c-api/complex.html

use std::ffi;

use rustpython_vm::{PyObject, PyObjectRef, builtins::PyComplex};

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct CPyComplex {
    pub real: ffi::c_double,
    pub imag: ffi::c_double,
}

impl From<CPyComplex> for PyComplex {
    fn from(value: CPyComplex) -> Self {
        PyComplex::new(num_complex::Complex64::new(value.real, value.imag))
    }
}

impl From<CPyComplex> for num_complex::Complex64 {
    fn from(value: CPyComplex) -> Self {
        num_complex::Complex64::new(value.real, value.imag)
    }
}

impl From<PyComplex> for CPyComplex {
    fn from(value: PyComplex) -> Self {
        let complex = value.to_complex();
        CPyComplex {
            real: complex.re,
            imag: complex.im,
        }
    }
}

impl From<num_complex::Complex64> for CPyComplex {
    fn from(value: num_complex::Complex64) -> Self {
        CPyComplex {
            real: value.re,
            imag: value.im,
        }
    }
}

// Associated functions for CPyComplex
// Always convert to PyComplex to do operations

#[unsafe(export_name = "_Py_c_sum")]
pub unsafe extern "C" fn c_sum(a: *const CPyComplex, b: *const CPyComplex) -> CPyComplex {
    let a: PyComplex = unsafe { *a }.into();
    let b: PyComplex = unsafe { *b }.into();
    (a.to_complex() + b.to_complex()).into()
}

#[unsafe(export_name = "_Py_c_diff")]
pub unsafe extern "C" fn c_diff(a: *const CPyComplex, b: *const CPyComplex) -> CPyComplex {
    let a: PyComplex = unsafe { *a }.into();
    let b: PyComplex = unsafe { *b }.into();
    (a.to_complex() - b.to_complex()).into()
}

#[unsafe(export_name = "_Py_c_neg")]
pub unsafe extern "C" fn c_neg(a: *const CPyComplex) -> CPyComplex {
    let a: PyComplex = unsafe { *a }.into();
    (-a.to_complex()).into()
}

#[unsafe(export_name = "_Py_c_prod")]
pub unsafe extern "C" fn c_prod(a: *const CPyComplex, b: *const CPyComplex) -> CPyComplex {
    let a: PyComplex = unsafe { *a }.into();
    let b: PyComplex = unsafe { *b }.into();
    (a.to_complex() * b.to_complex()).into()
}

#[unsafe(export_name = "_Py_c_quot")]
pub unsafe extern "C" fn c_quot(a: *const CPyComplex, b: *const CPyComplex) -> CPyComplex {
    let a: PyComplex = unsafe { *a }.into();
    let b: PyComplex = unsafe { *b }.into();
    (a.to_complex() / b.to_complex()).into()
}

#[unsafe(export_name = "_Py_c_pow")]
pub unsafe extern "C" fn c_pow(a: *const CPyComplex, b: *const CPyComplex) -> CPyComplex {
    let a: PyComplex = unsafe { *a }.into();
    let b: PyComplex = unsafe { *b }.into();
    (a.to_complex() * b.to_complex()).into()
}

#[unsafe(export_name = "PyComplex_FromCComplex")]
pub unsafe extern "C" fn complex_from_ccomplex(value: CPyComplex) -> *mut PyObject {
    let vm = crate::get_vm();
    Into::<PyObjectRef>::into(vm.ctx.new_complex(value.into()))
        .into_raw()
        .as_ptr()
}

#[unsafe(export_name = "PyComplex_FromDoubles")]
pub unsafe extern "C" fn complex_from_doubles(
    real: ffi::c_double,
    imag: ffi::c_double,
) -> *mut PyObject {
    let vm = crate::get_vm();
    Into::<PyObjectRef>::into(vm.ctx.new_complex(num_complex::Complex64::new(real, imag)))
        .into_raw()
        .as_ptr()
}

#[unsafe(export_name = "PyComplex_RealAsDouble")]
pub unsafe extern "C" fn complex_real_as_double(value: *mut PyObject) -> ffi::c_double {
    let vm = crate::get_vm();
    let value = crate::cast_obj_ptr(value).unwrap();
    let (complex, _) = value.try_complex(&vm).unwrap().unwrap();
    complex.re
}

#[unsafe(export_name = "PyComplex_ImagAsDouble")]
pub unsafe extern "C" fn complex_imag_as_double(value: *mut PyObject) -> ffi::c_double {
    let vm = crate::get_vm();
    let value = crate::cast_obj_ptr(value).unwrap();
    let (complex, _) = value.try_complex(&vm).unwrap().unwrap();
    complex.im
}

#[unsafe(export_name = "PyComplex_AsCComplex")]
pub unsafe extern "C" fn complex_as_ccomplex(value: *mut PyObject) -> CPyComplex {
    let vm = crate::get_vm();
    let value = crate::cast_obj_ptr(value).unwrap();
    let (complex, _) = value.try_complex(&vm).unwrap().unwrap();
    complex.into()
}
