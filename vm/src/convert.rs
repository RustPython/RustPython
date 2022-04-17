use crate::{
    pyobject::{AsPyObject, PyObject, PyObjectRef, PyRef, PyResult, PyValue},
    vm::VirtualMachine,
};

/// Marks a type that has the exact same layout as PyObjectRef, e.g. a type that is
/// `repr(transparent)` over PyObjectRef.
///
/// # Safety
/// Can only be implemented for types that are `repr(transparent)` over a PyObjectRef `obj`,
/// and logically valid so long as `check(vm, obj)` returns `Ok(())`
pub unsafe trait TransmuteFromObject: Sized {
    fn check(vm: &VirtualMachine, obj: &PyObject) -> PyResult<()>;
}

unsafe impl<T: PyValue> TransmuteFromObject for PyRef<T> {
    fn check(vm: &VirtualMachine, obj: &PyObject) -> PyResult<()> {
        let class = T::class(vm);
        if obj.isinstance(class) {
            if obj.payload_is::<T>() {
                Ok(())
            } else {
                Err(vm.new_downcast_runtime_error(class, obj))
            }
        } else {
            Err(vm.new_downcast_type_error(class, obj))
        }
    }
}

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
        T: PyValue,
        F: Fn(&T) -> PyResult<R>,
    {
        let class = T::class(vm);
        let special;
        let py_ref = if self.isinstance(class) {
            self.downcast_ref()
                .ok_or_else(|| vm.new_downcast_runtime_error(class, self))?
        } else {
            special = T::special_retrieve(vm, self)
                .unwrap_or_else(|| Err(vm.new_downcast_type_error(class, self)))?;
            &special
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
    T: PyValue,
{
    #[inline]
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        let class = T::class(vm);
        if obj.isinstance(class) {
            obj.downcast()
                .map_err(|obj| vm.new_downcast_runtime_error(class, obj))
        } else {
            T::special_retrieve(vm, &obj)
                .unwrap_or_else(|| Err(vm.new_downcast_type_error(class, obj)))
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
