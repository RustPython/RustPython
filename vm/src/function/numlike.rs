use crate::{PyObjectRef, PyResult, TryFromObject, TypeProtocol, VirtualMachine};
use num_complex::Complex64;

#[derive(Debug, Copy, Clone, PartialEq)]
#[repr(transparent)]
pub struct ArgComplexLike {
    value: Complex64,
}

impl ArgComplexLike {
    pub fn to_complex(self) -> Complex64 {
        self.value
    }
}

impl TryFromObject for ArgComplexLike {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        // We do not care if it was already a complex.
        let (value, _) = obj.try_complex(vm)?.ok_or_else(|| {
            vm.new_type_error(format!("must be real number, not {}", obj.class().name()))
        })?;
        Ok(ArgComplexLike { value })
    }
}

#[derive(Debug, Copy, Clone, PartialEq)]
#[repr(transparent)]
pub struct ArgFloatLike {
    value: f64,
}

impl ArgFloatLike {
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

impl TryFromObject for ArgFloatLike {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        let value = obj.try_to_f64(vm)?.ok_or_else(|| {
            vm.new_type_error(format!("must be real number, not {}", obj.class().name()))
        })?;
        Ok(ArgFloatLike { value })
    }
}

#[derive(Debug, Default, Copy, Clone, PartialEq)]
pub struct ArgBoolLike {
    value: bool,
}

impl ArgBoolLike {
    pub const TRUE: ArgBoolLike = ArgBoolLike { value: true };
    pub const FALSE: ArgBoolLike = ArgBoolLike { value: false };

    pub fn to_bool(self) -> bool {
        self.value
    }
}

impl TryFromObject for ArgBoolLike {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        Ok(ArgBoolLike {
            value: obj.try_to_bool(vm)?,
        })
    }
}
