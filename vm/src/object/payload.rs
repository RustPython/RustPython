use super::{MaybeTraverse, Py, PyObjectBuilder, PyObjectRef, PyRef, PyResult, core::SuperPayload};
use crate::{
    PyRefExact,
    builtins::{PyType, PyTypeRef},
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
    #[allow(private_bounds)]
    type Super: SuperPayload;

    fn class(ctx: &Context) -> &'static Py<PyType>;

    #[inline]
    fn into_pyobject(self, vm: &VirtualMachine) -> PyObjectRef
    where
        Self::Super: SuperPyDefault,
    {
        self.into_ref(&vm.ctx).into()
    }

    #[inline]
    fn into_exact_ref(self, ctx: &Context) -> PyRefExact<Self>
    where
        Self::Super: SuperPyDefault,
    {
        PyObjectBuilder::new(self).build_exact(ctx)
    }

    #[inline]
    fn into_ref(self, ctx: &Context) -> PyRef<Self>
    where
        Self::Super: SuperPyDefault,
    {
        PyObjectBuilder::new(self).build(ctx)
    }

    #[inline]
    fn into_ref_with_type(self, vm: &VirtualMachine, cls: PyTypeRef) -> PyResult<PyRef<Self>>
    where
        Self::Super: SuperPyDefault,
    {
        PyObjectBuilder::new(self).build_with_type(cls, vm)
    }
}

pub use PyPayload as PyObjectPayload;

pub trait PyDefault {
    fn py_default(ctx: &Context) -> Self;
}

impl<T: Default> PyDefault for T {
    fn py_default(_ctx: &Context) -> Self {
        T::default()
    }
}

/// Implemented for `PyPayload::Super` types that implement [`PyDefault`].
pub trait SuperPyDefault: SuperPayload {
    #[doc(hidden)]
    fn py_from_header(header: super::PyObjHeader, ctx: &Context) -> Self::Repr;
}

/// Implemented for `PyPayload::Super` types that implement [`Default`].
pub trait SuperDefault: SuperPayload + SuperPyDefault {
    #[doc(hidden)]
    fn from_header(header: super::PyObjHeader) -> Self::Repr;
}
