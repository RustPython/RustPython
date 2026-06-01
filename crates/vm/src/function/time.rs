use crate::{PyObjectRef, PyResult, TryFromObject, VirtualMachine};

/// A Python timeout value that accepts both `float` and `int`.
///
/// `TimeoutSeconds` implements `FromArgs` so that a built-in function can accept
/// timeout parameters given as either `float` or `int`, normalizing them to `f64`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimeoutSeconds {
    value: f64,
}

impl TimeoutSeconds {
    #[must_use]
    pub const fn new(secs: f64) -> Self {
        Self { value: secs }
    }

    #[inline]
    #[must_use]
    pub fn to_secs_f64(self) -> f64 {
        self.value
    }
}

impl TryFromObject for TimeoutSeconds {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        let value = match super::Either::<f64, i64>::try_from_object(vm, obj)? {
            super::Either::A(f) => f,
            super::Either::B(i) => i as f64,
        };
        if value.is_nan() {
            return Err(vm.new_value_error("Invalid value NaN (not a number)".to_owned()));
        }
        Ok(Self { value })
    }
}
