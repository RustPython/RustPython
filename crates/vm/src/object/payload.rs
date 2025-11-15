use crate::object::{MaybeTraverse, Py, PyObjectRef, PyRef, PyResult};
use crate::{
    PyObject, PyRefExact,
    builtins::{PyBaseExceptionRef, PyType, PyTypeRef},
    types::PyTypeFlags,
    vm::{Context, VirtualMachine},
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

pub trait PyPayload:
    std::fmt::Debug + MaybeTraverse + PyThreadingConstraint + Sized + 'static
{
    #[inline]
    fn payload_type_id() -> std::any::TypeId {
        std::any::TypeId::of::<Self>()
    }

    /// # Safety: this function should only be called if `payload_type_id` matches the type of `obj`.
    #[inline]
    fn downcastable_from(obj: &PyObject) -> bool {
        obj.typeid() == Self::payload_type_id()
    }

    fn try_downcast_from(obj: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
        if Self::downcastable_from(obj) {
            return Ok(());
        }

        #[cold]
        fn raise_downcast_type_error(
            vm: &VirtualMachine,
            class: &Py<PyType>,
            obj: &PyObject,
        ) -> PyBaseExceptionRef {
            vm.new_downcast_type_error(class, obj)
        }

        let class = Self::class(&vm.ctx);
        Err(raise_downcast_type_error(vm, class, obj))
    }

    fn class(ctx: &Context) -> &'static Py<PyType>;

    #[inline]
    fn into_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        self.into_ref(&vm.ctx).into()
    }

    #[inline]
    fn _into_ref(self, cls: PyTypeRef, ctx: &Context) -> PyRef<Self> {
        let dict = if cls.slots.flags.has_feature(PyTypeFlags::HAS_DICT) {
            Some(ctx.new_dict())
        } else {
            None
        };
        PyRef::new_ref(self, cls, dict)
    }

    #[inline]
    fn into_exact_ref(self, ctx: &Context) -> PyRefExact<Self> {
        unsafe {
            // Self::into_ref() always returns exact typed PyRef
            PyRefExact::new_unchecked(self.into_ref(ctx))
        }
    }

    #[inline]
    fn into_ref(self, ctx: &Context) -> PyRef<Self> {
        let cls = Self::class(ctx);
        self._into_ref(cls.to_owned(), ctx)
    }

    #[inline]
    fn into_ref_with_type(self, vm: &VirtualMachine, cls: PyTypeRef) -> PyResult<PyRef<Self>> {
        let exact_class = Self::class(&vm.ctx);
        if cls.fast_issubclass(exact_class) {
            Ok(self._into_ref(cls, &vm.ctx))
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
    PyPayload + std::any::Any + std::fmt::Debug + MaybeTraverse + PyThreadingConstraint + 'static
{
}

impl<T: PyPayload + 'static> PyObjectPayload for T {}

pub trait SlotOffset {
    fn offset() -> usize;
}
