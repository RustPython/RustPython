use crate::{
    pyobject::{AsObject, PyObject, PyRef, PyResult, PyValue},
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
        if obj.fast_isinstance(class) {
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
