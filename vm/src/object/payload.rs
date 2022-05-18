use super::{Py, PyObject, PyObjectRef, PyRef, PyResult};
use crate::{
    builtins::{PyBaseExceptionRef, PyType, PyTypeRef},
    types::PyTypeFlags,
    vm::VirtualMachine,
};

cfg_if::cfg_if! {
    if #[cfg(feature = "threading")] {
        pub trait PyThreadingConstraint: Send + Sync {}
        impl<T: Send + Sync> PyThreadingConstraint for T {}
    } else {
        pub trait PyThreadingConstraint {}
        impl<T> PyThreadingConstraint for T {}
    }
}

pub trait PyPayload: std::fmt::Debug + PyThreadingConstraint + Sized + 'static {
    fn class(vm: &VirtualMachine) -> &'static Py<PyType>;

    #[inline]
    fn into_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        self.into_ref(vm).into()
    }

    #[inline(always)]
    fn special_retrieve(_vm: &VirtualMachine, _obj: &PyObject) -> Option<PyResult<PyRef<Self>>> {
        None
    }

    #[inline]
    fn _into_ref(self, cls: PyTypeRef, vm: &VirtualMachine) -> PyRef<Self> {
        let dict = if cls.slots.flags.has_feature(PyTypeFlags::HAS_DICT) {
            Some(vm.ctx.new_dict())
        } else {
            None
        };
        PyRef::new_ref(self, cls, dict)
    }

    #[inline]
    fn into_ref(self, vm: &VirtualMachine) -> PyRef<Self> {
        let cls = Self::class(vm);
        self._into_ref(cls.to_owned(), vm)
    }

    #[inline]
    fn into_ref_with_type(self, vm: &VirtualMachine, cls: PyTypeRef) -> PyResult<PyRef<Self>> {
        let exact_class = Self::class(vm);
        if cls.fast_issubclass(exact_class) {
            Ok(self._into_ref(cls, vm))
        } else {
            #[cold]
            #[inline(never)]
            fn _into_ref_with_type_error(
                vm: &VirtualMachine,
                cls: &PyTypeRef,
                exact_class: &Py<PyType>,
            ) -> PyBaseExceptionRef {
                vm.new_type_error(format!(
                    "'{}' is not a subtype of '{}'",
                    &cls.name(),
                    exact_class.name()
                ))
            }
            Err(_into_ref_with_type_error(vm, &cls, exact_class))
        }
    }
}

pub trait PyObjectPayload:
    std::any::Any + std::fmt::Debug + PyThreadingConstraint + 'static
{
}

impl<T: PyPayload + 'static> PyObjectPayload for T {}
