use crate::common::rc::{PyRc, PyWeak};
use crate::pyobject::{IdProtocol, PyObject, PyObjectPayload};
use std::borrow;
use std::fmt;
use std::ops::Deref;

pub struct PyObjectRc<T = dyn PyObjectPayload>
where
    T: ?Sized + PyObjectPayload,
    PyRc<PyObject<T>>: AsPyObjectRef,
{
    inner: PyRc<PyObject<T>>,
}

pub type PyObjectWeak<T = dyn PyObjectPayload> = PyWeak<PyObject<T>>;

pub trait AsPyObjectRef {
    fn _as_ref(self) -> PyRc<PyObject<dyn PyObjectPayload>>;
}

impl<T> AsPyObjectRef for PyRc<PyObject<T>>
where
    T: PyObjectPayload,
{
    fn _as_ref(self) -> PyRc<PyObject<dyn PyObjectPayload>> {
        self
    }
}

impl AsPyObjectRef for PyRc<PyObject<dyn PyObjectPayload>> {
    fn _as_ref(self) -> PyRc<PyObject<dyn PyObjectPayload>> {
        self
    }
}

impl<T> PyObjectRc<T>
where
    T: ?Sized + PyObjectPayload,
    PyRc<PyObject<T>>: AsPyObjectRef,
{
    pub fn into_raw(this: Self) -> *const PyObject<T> {
        let ptr = PyRc::as_ptr(&this.inner);
        std::mem::forget(this);
        ptr
    }

    unsafe fn into_rc(this: Self) -> PyRc<PyObject<T>> {
        std::mem::transmute(this)
    }

    pub fn into_ref(this: Self) -> PyObjectRc<dyn PyObjectPayload> {
        PyObjectRc::<dyn PyObjectPayload> {
            inner: unsafe { Self::into_rc(this) }._as_ref(),
        }
    }

    /// # Safety
    /// See PyRc::from_raw
    pub unsafe fn from_raw(ptr: *const PyObject<T>) -> Self {
        Self {
            inner: PyRc::from_raw(ptr),
        }
    }

    pub fn new(value: PyObject<T>) -> Self
    where
        T: Sized,
    {
        Self {
            inner: PyRc::new(value),
        }
    }

    pub fn strong_count(this: &Self) -> usize {
        PyRc::strong_count(&this.inner)
    }

    pub fn weak_count(this: &Self) -> usize {
        PyRc::weak_count(&this.inner)
    }

    pub fn downgrade(this: &Self) -> PyObjectWeak<T> {
        PyRc::downgrade(&this.inner)
    }

    pub fn upgrade_weak(weak: &PyObjectWeak<T>) -> Option<Self> {
        weak.upgrade().map(|inner| PyObjectRc { inner })
    }
}

#[cfg(feature = "threading")]
unsafe impl<T> Send for PyObjectRc<T>
where
    T: ?Sized + PyObjectPayload,
    PyRc<PyObject<T>>: AsPyObjectRef,
{
}
#[cfg(feature = "threading")]
unsafe impl<T> Sync for PyObjectRc<T>
where
    T: ?Sized + PyObjectPayload,
    PyRc<PyObject<T>>: AsPyObjectRef,
{
}

impl<T> Deref for PyObjectRc<T>
where
    T: ?Sized + PyObjectPayload,
    PyRc<PyObject<T>>: AsPyObjectRef,
{
    type Target = PyObject<T>;

    #[inline]
    fn deref(&self) -> &PyObject<T> {
        self.inner.deref()
    }
}

impl<T> Clone for PyObjectRc<T>
where
    T: ?Sized + PyObjectPayload,
    PyRc<PyObject<T>>: AsPyObjectRef,
{
    fn clone(&self) -> Self {
        PyObjectRc {
            inner: self.inner.clone(),
        }
    }
}

impl<T> fmt::Display for PyObjectRc<T>
where
    T: ?Sized + PyObjectPayload,
    PyRc<PyObject<T>>: AsPyObjectRef,
    PyObject<T>: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.inner.fmt(f)
    }
}

impl<T> fmt::Debug for PyObjectRc<T>
where
    T: ?Sized + PyObjectPayload,
    PyRc<PyObject<T>>: AsPyObjectRef,
    PyObject<T>: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.inner.fmt(f)
    }
}

impl<T> fmt::Pointer for PyObjectRc<T>
where
    T: ?Sized + PyObjectPayload,
    PyRc<PyObject<T>>: AsPyObjectRef,
    PyObject<T>: fmt::Pointer,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.inner.fmt(f)
    }
}

impl<T> borrow::Borrow<T> for PyObjectRc<T>
where
    T: ?Sized + PyObjectPayload,
    PyRc<PyObject<T>>: AsPyObjectRef + borrow::Borrow<T>,
{
    fn borrow(&self) -> &T {
        self.inner.borrow()
    }
}

impl<T> AsRef<T> for PyObjectRc<T>
where
    T: ?Sized + PyObjectPayload,
    PyRc<PyObject<T>>: AsPyObjectRef + AsRef<T>,
{
    fn as_ref(&self) -> &T {
        self.inner.as_ref()
    }
}

impl IdProtocol for PyObjectRc {
    fn get_id(&self) -> usize {
        self.inner.get_id()
    }
}
