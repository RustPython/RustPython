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
