use crate::{
    Py, VirtualMachine,
    builtins::PyFloat,
    object::{AsObject, PyObject, PyObjectRef, PyPayload, PyRef, PyResult},
};
use malachite_bigint::Sign;
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
impl<T: for<'a> TryFromBorrowedObject<'a>> TryFromObject for T {
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
    pub fn try_to_value<'a, T>(&'a self, vm: &VirtualMachine) -> PyResult<T>
    where
        T: 'a + TryFromBorrowedObject<'a>,
    {
        T::try_from_borrowed_object(vm, self)
    }

    pub fn try_to_ref<'a, T>(&'a self, vm: &VirtualMachine) -> PyResult<&'a Py<T>>
    where
        T: 'a + PyPayload,
    {
        self.try_to_value::<&Py<T>>(vm)
    }

    pub fn try_value_with<T, F, R>(&self, f: F, vm: &VirtualMachine) -> PyResult<R>
    where
        T: PyPayload,
        F: Fn(&T) -> PyResult<R>,
    {
        let class = T::class(&vm.ctx);
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
pub trait TryFromBorrowedObject<'a>: Sized
where
    Self: 'a,
{
    /// Attempt to convert a Python object to a value of this type.
    fn try_from_borrowed_object(vm: &VirtualMachine, obj: &'a PyObject) -> PyResult<Self>;
}

impl<T> TryFromObject for PyRef<T>
where
    T: PyPayload,
{
    #[inline]
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        let class = T::class(&vm.ctx);
        if obj.fast_isinstance(class) {
            T::try_downcast_from(&obj, vm)?;
            Ok(unsafe { obj.downcast_unchecked() })
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

impl<'a, T: 'a + TryFromObject> TryFromBorrowedObject<'a> for Vec<T> {
    fn try_from_borrowed_object(vm: &VirtualMachine, value: &'a PyObject) -> PyResult<Self> {
        vm.extract_elements_with(value, |obj| T::try_from_object(vm, obj))
    }
}

impl<'a, T: PyPayload> TryFromBorrowedObject<'a> for &'a Py<T> {
    fn try_from_borrowed_object(vm: &VirtualMachine, obj: &'a PyObject) -> PyResult<Self> {
        let class = T::class(&vm.ctx);
        if obj.fast_isinstance(class) {
            obj.downcast_ref()
                .ok_or_else(|| vm.new_downcast_runtime_error(class, &obj))
        } else {
            Err(vm.new_downcast_type_error(class, &obj))
        }
    }
}

impl TryFromObject for std::time::Duration {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        if let Some(float) = obj.downcast_ref::<PyFloat>() {
            let f = float.to_f64();
            if f < 0.0 {
                return Err(vm.new_value_error("negative duration"));
            }
            Ok(Self::from_secs_f64(f))
        } else if let Some(int) = obj.try_index_opt(vm) {
            let int = int?;
            let bigint = int.as_bigint();
            if bigint.sign() == Sign::Minus {
                return Err(vm.new_value_error("negative duration"));
            }

            let sec = bigint
                .to_u64()
                .ok_or_else(|| vm.new_value_error("value out of range"))?;
            Ok(Self::from_secs(sec))
        } else {
            Err(vm.new_type_error(format!(
                "expected an int or float for duration, got {}",
                obj.class()
            )))
        }
    }
}
