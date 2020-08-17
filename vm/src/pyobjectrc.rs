use crate::common::rc::{PyRc, PyWeak};
use crate::pyobject::{DynPyObject, IdProtocol, PyObject, PyObjectPayload, PyRef, PyValue};
use crate::vm::VirtualMachine;
use std::borrow;
use std::convert::From;
use std::fmt;
use std::ops::Deref;

#[derive(Clone)]
pub struct PyObjectRc {
    inner: PyRc<DynPyObject>,
}

pub type PyObjectWeak = PyWeak<DynPyObject>;

type InnerRc = PyRc<DynPyObject>;

impl PyObjectRc {
    /// # Safety
    /// if rc is dropped without wrapping again, drop will not be called
    unsafe fn into_rc(this: Self) -> PyRc<DynPyObject> {
        std::mem::transmute(this)
    }

    pub fn into_raw(this: Self) -> *const DynPyObject {
        PyRc::into_raw(unsafe { Self::into_rc(this) })
    }

    /// # Safety
    /// See PyRc::from_raw
    pub unsafe fn from_raw(ptr: *const DynPyObject) -> Self {
        PyRc::from_raw(ptr).into()
    }

    pub fn strong_count(this: &Self) -> usize {
        InnerRc::strong_count(this)
    }

    pub fn weak_count(this: &Self) -> usize {
        InnerRc::weak_count(this)
    }

    pub fn downgrade(this: &Self) -> PyObjectWeak {
        InnerRc::downgrade(&this.inner)
    }

    pub fn upgrade_weak(weak: &PyObjectWeak) -> Option<Self> {
        weak.upgrade().map(|inner| PyObjectRc { inner })
    }
}

#[cfg(feature = "threading")]
unsafe impl Send for PyObjectRc {}
#[cfg(feature = "threading")]
unsafe impl Sync for PyObjectRc {}

impl From<PyRc<DynPyObject>> for PyObjectRc {
    fn from(rc: PyRc<DynPyObject>) -> PyObjectRc {
        Self { inner: rc }
    }
}

impl<P: PyObjectPayload> From<PyRc<PyObject<P>>> for PyObjectRc {
    fn from(rc: PyRc<PyObject<P>>) -> PyObjectRc {
        Self { inner: rc }
    }
}

impl Deref for PyObjectRc {
    type Target = PyRc<DynPyObject>;

    #[inline]
    fn deref(&self) -> &PyRc<DynPyObject> {
        &self.inner
    }
}

impl fmt::Display for PyObjectRc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.inner.fmt(f)
    }
}

impl fmt::Debug for PyObjectRc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.inner.fmt(f)
    }
}

impl fmt::Pointer for PyObjectRc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.inner.fmt(f)
    }
}

impl borrow::Borrow<DynPyObject> for PyObjectRc {
    fn borrow(&self) -> &DynPyObject {
        self.inner.borrow()
    }
}

impl borrow::Borrow<PyRc<DynPyObject>> for PyObjectRc {
    fn borrow(&self) -> &PyRc<DynPyObject> {
        &self.inner
    }
}

impl AsRef<DynPyObject> for PyObjectRc {
    fn as_ref(&self) -> &DynPyObject {
        self.inner.as_ref()
    }
}

impl IdProtocol for PyObjectRc {
    fn get_id(&self) -> usize {
        self.inner.get_id()
    }
}

impl PyObjectRc {
    pub fn downcast<T: PyObjectPayload + PyValue>(self) -> Result<PyRef<T>, Self> {
        unsafe { PyObjectRc::into_rc(self) }.downcast()
    }

    pub fn downcast_exact<T: PyObjectPayload + PyValue>(
        self,
        vm: &VirtualMachine,
    ) -> Result<PyRef<T>, Self> {
        unsafe { PyObjectRc::into_rc(self) }.downcast_exact(vm)
    }

    pub fn downcast_generic<T: PyObjectPayload>(self) -> Result<PyRc<PyObject<T>>, Self> {
        unsafe { PyObjectRc::into_rc(self) }.downcast_generic()
    }
}
