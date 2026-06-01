use crate::object::{MaybeTraverse, Py, PyObjectRef, PyRef, PyResult};
use crate::{
    PyObject, PyRefExact,
    builtins::{PyBaseExceptionRef, PyType, PyTypeRef},
    types::PyTypeFlags,
    vm::{Context, VirtualMachine},
};
use core::ptr::NonNull;

cfg_select! {
    feature = "threading" => {
        pub trait PyThreadingConstraint: Send + Sync {}
        impl<T: Send + Sync> PyThreadingConstraint for T {}
    }
    _ => {
        pub trait PyThreadingConstraint {}
        impl<T> PyThreadingConstraint for T {}
    }
}

#[cold]
pub(crate) fn cold_downcast_type_error(
    vm: &VirtualMachine,
    class: &Py<PyType>,
    obj: &PyObject,
) -> PyBaseExceptionRef {
    vm.new_downcast_type_error(class, obj)
}

pub trait PyPayload: MaybeTraverse + PyThreadingConstraint + Sized + 'static {
    const PAYLOAD_TYPE_ID: core::any::TypeId = core::any::TypeId::of::<Self>();

    /// # Safety
    /// This function should only be called if `payload_type_id` matches the type of `obj`.
    #[inline]
    unsafe fn validate_downcastable_from(_obj: &PyObject) -> bool {
        true
    }

    fn try_downcast_from(obj: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
        if obj.downcastable::<Self>() {
            return Ok(());
        }

        let class = Self::class(&vm.ctx);
        Err(cold_downcast_type_error(vm, class, obj))
    }

    fn class(ctx: &Context) -> &'static Py<PyType>;

    /// Whether this type has a freelist. Types with freelists require
    /// immediate (non-deferred) GC untracking during dealloc to prevent
    /// race conditions when the object is reused.
    const HAS_FREELIST: bool = false;

    /// Maximum number of objects to keep in the freelist.
    const MAX_FREELIST: usize = 0;

    /// Try to push a dead object onto this type's freelist for reuse.
    /// Returns true if the object was stored (caller must NOT free the memory).
    /// Called before tp_clear, so the payload is still intact.
    ///
    /// # Safety
    /// `obj` must be a valid pointer to a `PyInner<Self>` with refcount 0.
    /// The payload is still initialized and can be read for bucket selection.
    #[inline]
    unsafe fn freelist_push(_obj: *mut PyObject) -> bool {
        false
    }

    /// Try to pop a pre-allocated object from this type's freelist.
    /// The returned pointer still has the old payload; the caller must
    /// reinitialize `ref_count`, `gc_bits`, and `payload`.
    ///
    /// # Safety
    /// The returned pointer (if Some) must point to a valid `PyInner<Self>`
    /// whose payload is still initialized from a previous allocation. The caller
    /// will drop and overwrite `payload` before reuse.
    #[inline]
    unsafe fn freelist_pop(_payload: &Self) -> Option<NonNull<PyObject>> {
        None
    }

    #[inline]
    fn into_pyobject(self, vm: &VirtualMachine) -> PyObjectRef
    where
        Self: core::fmt::Debug,
    {
        self.into_ref(&vm.ctx).into()
    }

    #[inline]
    fn _into_ref(self, cls: PyTypeRef, ctx: &Context) -> PyRef<Self>
    where
        Self: core::fmt::Debug,
    {
        let dict = if cls.slots.flags.has_feature(PyTypeFlags::HAS_DICT) {
            Some(ctx.new_dict())
        } else {
            None
        };
        PyRef::new_ref(self, cls, dict)
    }

    #[inline]
    fn into_exact_ref(self, ctx: &Context) -> PyRefExact<Self>
    where
        Self: core::fmt::Debug,
    {
        unsafe {
            // Self::into_ref() always returns exact typed PyRef
            PyRefExact::new_unchecked(self.into_ref(ctx))
        }
    }

    #[inline]
    fn into_ref(self, ctx: &Context) -> PyRef<Self>
    where
        Self: core::fmt::Debug,
    {
        let cls = Self::class(ctx);
        self._into_ref(cls.to_owned(), ctx)
    }

    #[inline]
    fn into_ref_with_type(self, vm: &VirtualMachine, cls: PyTypeRef) -> PyResult<PyRef<Self>>
    where
        Self: core::fmt::Debug,
    {
        let exact_class = Self::class(&vm.ctx);
        if cls.fast_issubclass(exact_class) {
            if exact_class.slots.basicsize != cls.slots.basicsize {
                #[cold]
                #[inline(never)]
                fn _into_ref_size_error(
                    vm: &VirtualMachine,
                    cls: &Py<PyType>,
                    exact_class: &Py<PyType>,
                ) -> PyBaseExceptionRef {
                    vm.new_type_error(format!(
                        "cannot create '{}' instance: size differs from base type '{}'",
                        cls.name(),
                        exact_class.name()
                    ))
                }
                return Err(_into_ref_size_error(vm, &cls, exact_class));
            }
            Ok(self._into_ref(cls, &vm.ctx))
        } else {
            #[cold]
            #[inline(never)]
            fn _into_ref_with_type_error(
                vm: &VirtualMachine,
                cls: &Py<PyType>,
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
    PyPayload + core::any::Any + core::fmt::Debug + MaybeTraverse + PyThreadingConstraint + 'static
{
}

impl<T: PyPayload + core::fmt::Debug + 'static> PyObjectPayload for T {}

pub trait SlotOffset {
    fn offset() -> usize;
}
