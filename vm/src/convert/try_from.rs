use crate::{
    builtins::PyFloat,
    object::{AsObject, PyObject, PyObjectRef, PyPayload, PyRef, PyResult},
    vm::VirtualMachine,
};
use num_traits::ToPrimitive;

/// Implemented by any type that can be created from a Python object.
///
/// Any type that implements `TryFromObject` is automatically `FromArgs`, and
/// so can be accepted as a argument to a built-in function.
pub trait TryFromObject: Sized {
    /// Attempt to convert a Python object to a value of this type.
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self>;
}

/// Rust-side only version of TryFromObject to reduce unnecessary Rc::clone
impl<T: TryFromBorrowedObject> TryFromObject for T {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        TryFromBorrowedObject::try_from_borrowed_object(vm, &obj)
    }
}

impl PyObjectRef {
    pub fn try_into_value<T>(self, vm: &VirtualMachine) -> PyResult<T>
    where
        T: TryFromObject,
    {
        T::try_from_object(vm, self)
    }
}

impl PyObject {
    pub fn try_to_value<T>(&self, vm: &VirtualMachine) -> PyResult<T>
    where
        T: TryFromBorrowedObject,
    {
        T::try_from_borrowed_object(vm, self)
    }

    pub fn try_value_with<T, F, R>(&self, f: F, vm: &VirtualMachine) -> PyResult<R>
    where
        T: PyPayload,
        F: Fn(&T) -> PyResult<R>,
    {
        let class = T::class(vm);
        let py_ref = if self.fast_isinstance(class) {
            self.downcast_ref()
                .ok_or_else(|| vm.new_downcast_runtime_error(class, self))?
        } else {
            return Err(vm.new_downcast_type_error(class, self));
        };
        f(py_ref)
    }
}

/// Lower-cost variation of `TryFromObject`
pub trait TryFromBorrowedObject: Sized {
    /// Attempt to convert a Python object to a value of this type.
    fn try_from_borrowed_object(vm: &VirtualMachine, obj: &PyObject) -> PyResult<Self>;
}

impl<T> TryFromObject for PyRef<T>
where
    T: PyPayload,
{
    #[inline]
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        let class = T::class(vm);
        if obj.fast_isinstance(class) {
            obj.downcast()
                .map_err(|obj| vm.new_downcast_runtime_error(class, &obj))
        } else {
            Err(vm.new_downcast_type_error(class, &obj))
        }
    }
}

impl TryFromObject for PyObjectRef {
    #[inline]
    fn try_from_object(_vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        Ok(obj)
    }
}

impl<T: TryFromObject> TryFromObject for Option<T> {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        if vm.is_none(&obj) {
            Ok(None)
        } else {
            T::try_from_object(vm, obj).map(Some)
        }
    }
}

impl<T: TryFromObject> TryFromBorrowedObject for Vec<T> {
    fn try_from_borrowed_object(vm: &VirtualMachine, value: &PyObject) -> PyResult<Self> {
        vm.extract_elements_with(value, |obj| T::try_from_object(vm, obj))
    }
}

impl TryFromObject for std::time::Duration {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        use std::time::Duration;
        if let Some(float) = obj.payload::<PyFloat>() {
            Ok(Duration::from_secs_f64(float.to_f64()))
        } else if let Some(int) = obj.try_index_opt(vm) {
            let sec = int?
                .as_bigint()
                .to_u64()
                .ok_or_else(|| vm.new_value_error("value out of range".to_owned()))?;
            Ok(Duration::from_secs(sec))
        } else {
            Err(vm.new_type_error(format!(
                "expected an int or float for duration, got {}",
                obj.class()
            )))
        }
    }
}
