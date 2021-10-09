use crate::{PyObjectRef, PyResult, TryFromObject, TypeProtocol, VirtualMachine};
use num_complex::Complex64;

/// A Python complex-like object.
///
/// `ArgIntoComplex` implements `FromArgs` so that a built-in function can accept
/// any object that can be transformed into a complex.
///
/// If the object is not a Python complex object but has a `__complex__()`
/// method, this method will first be called to convert the object into a float.
/// If `__complex__()` is not defined then it falls back to `__float__()`. If
/// `__float__()` is not defined it falls back to `__index__()`.
#[derive(Debug, Copy, Clone, PartialEq)]
#[repr(transparent)]
pub struct ArgIntoComplex {
    value: Complex64,
}

impl ArgIntoComplex {
    pub fn to_complex(self) -> Complex64 {
        self.value
    }
}

impl TryFromObject for ArgIntoComplex {
    // Equivalent to PyComplex_AsCComplex
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        // We do not care if it was already a complex.
        let (value, _) = obj.try_complex(vm)?.ok_or_else(|| {
            vm.new_type_error(format!("must be real number, not {}", obj.class().name()))
        })?;
        Ok(ArgIntoComplex { value })
    }
}

/// A Python float-like object.
///
/// `ArgIntoFloat` implements `FromArgs` so that a built-in function can accept
/// any object that can be transformed into a float.
///
/// If the object is not a Python floating point object but has a `__float__()`
/// method, this method will first be called to convert the object into a float.
/// If `__float__()` is not defined then it falls back to `__index__()`.
#[derive(Debug, Copy, Clone, PartialEq)]
#[repr(transparent)]
pub struct ArgIntoFloat {
    value: f64,
}

impl ArgIntoFloat {
    pub fn to_f64(self) -> f64 {
        self.value
    }

    pub fn vec_into_f64(v: Vec<Self>) -> Vec<f64> {
        // TODO: Vec::into_raw_parts once stabilized
        let mut v = std::mem::ManuallyDrop::new(v);
        let (p, l, c) = (v.as_mut_ptr(), v.len(), v.capacity());
        // SAFETY: IntoPyFloat is repr(transparent) over f64
        unsafe { Vec::from_raw_parts(p.cast(), l, c) }
    }
}

impl TryFromObject for ArgIntoFloat {
    // Equivalent to PyFloat_AsDouble.
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        let value = obj.try_to_f64(vm)?.ok_or_else(|| {
            vm.new_type_error(format!("must be real number, not {}", obj.class().name()))
        })?;
        Ok(ArgIntoFloat { value })
    }
}

/// A Python bool-like object.
///
/// `ArgIntoBool` implements `FromArgs` so that a built-in function can accept
/// any object that can be transformed into a boolean.
///
/// By default an object is considered true unless its class defines either a
/// `__bool__()` method that returns False or a `__len__()` method that returns
/// zero, when called with the object.
#[derive(Debug, Default, Copy, Clone, PartialEq)]
pub struct ArgIntoBool {
    value: bool,
}

impl ArgIntoBool {
    pub const TRUE: ArgIntoBool = ArgIntoBool { value: true };
    pub const FALSE: ArgIntoBool = ArgIntoBool { value: false };

    pub fn to_bool(self) -> bool {
        self.value
    }
}

impl TryFromObject for ArgIntoBool {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        Ok(ArgIntoBool {
            value: obj.try_to_bool(vm)?,
        })
    }
}
