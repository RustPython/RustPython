use crate::{
    builtins::{PyStr, PyStrInterned, PyTypeRef},
    common::lock::PyRwLock,
    convert::ToPyObject,
    AsObject, Py, PyExact, PyObject, PyObjectRef, PyPayload, PyRef, PyRefExact, VirtualMachine,
};
use std::{
    borrow::{Borrow, ToOwned},
    ops::Deref,
};

#[derive(Debug)]
pub struct StringPool {
    inner: PyRwLock<std::collections::HashSet<CachedPyStrRef, ahash::RandomState>>,
}

impl Default for StringPool {
    fn default() -> Self {
        Self {
            inner: PyRwLock::new(Default::default()),
        }
    }
}

impl Clone for StringPool {
    fn clone(&self) -> Self {
        Self {
            inner: PyRwLock::new(self.inner.read().clone()),
        }
    }
}

impl StringPool {
    #[inline]
    pub unsafe fn intern<S: Internable>(&self, s: S, typ: PyTypeRef) -> &'static PyStrInterned {
        if let Some(found) = self.interned(s.as_ref()) {
            return found;
        }

        #[cold]
        fn miss(zelf: &StringPool, s: PyRefExact<PyStr>) -> &'static PyStrInterned {
            let cache = CachedPyStrRef { inner: s };
            let inserted = zelf.inner.write().insert(cache.clone());
            if inserted {
                let interned = unsafe { cache.as_interned_str() };
                unsafe { interned.as_object().mark_intern() };
                interned
            } else {
                unsafe {
                    zelf.inner
                        .read()
                        .get(cache.as_ref())
                        .expect("inserted is false")
                        .as_interned_str()
                }
            }
        }
        let str_ref = s.into_pyref_exact(typ);
        miss(self, str_ref)
    }

    #[inline]
    pub fn interned<S: MaybeInterned + ?Sized>(&self, s: &S) -> Option<&'static PyStrInterned> {
        if let Some(interned) = s.as_interned() {
            return Some(interned);
        }
        self.inner
            .read()
            .get(s.as_ref())
            .map(|cached| unsafe { cached.as_interned_str() })
    }
}

#[derive(Debug, Clone)]
#[repr(transparent)]
pub struct CachedPyStrRef {
    inner: PyRefExact<PyStr>,
}

impl std::hash::Hash for CachedPyStrRef {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.inner.as_str().hash(state)
    }
}

impl PartialEq for CachedPyStrRef {
    fn eq(&self, other: &Self) -> bool {
        self.inner.as_str() == other.inner.as_str()
    }
}

impl Eq for CachedPyStrRef {}

impl std::borrow::Borrow<str> for CachedPyStrRef {
    #[inline]
    fn borrow(&self) -> &str {
        self.inner.as_str()
    }
}

impl AsRef<str> for CachedPyStrRef {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl CachedPyStrRef {
    /// # Safety
    /// the given cache must be alive while returned reference is alive
    #[inline]
    unsafe fn as_interned_str(&self) -> &'static PyStrInterned {
        std::mem::transmute_copy(self)
    }

    #[inline]
    fn as_str(&self) -> &str {
        self.inner.as_str()
    }
}

pub struct PyInterned<T>
where
    T: PyPayload,
{
    inner: Py<T>,
}

impl<T: PyPayload> PyInterned<T> {
    #[inline]
    pub fn leak(cache: PyRef<T>) -> &'static Self {
        unsafe { std::mem::transmute(cache) }
    }

    #[inline]
    fn as_ptr(&self) -> *const Py<T> {
        self as *const _ as *const _
    }

    #[inline]
    pub fn to_owned(&'static self) -> PyRef<T> {
        unsafe { (*(&self as *const _ as *const PyRef<T>)).clone() }
    }

    #[inline]
    pub fn to_object(&'static self) -> PyObjectRef {
        self.to_owned().into()
    }
}

impl<T: PyPayload> Borrow<PyObject> for PyInterned<T> {
    #[inline(always)]
    fn borrow(&self) -> &PyObject {
        self.inner.borrow()
    }
}

// NOTE: std::hash::Hash of Self and Self::Borrowed *must* be the same
// This is ok only because PyObject doesn't implement Hash
impl<T: PyPayload> std::hash::Hash for PyInterned<T> {
    #[inline(always)]
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.get_id().hash(state)
    }
}

impl<T: PyPayload> Deref for PyInterned<T> {
    type Target = Py<T>;
    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<T: PyPayload> PartialEq for PyInterned<T> {
    #[inline(always)]
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }
}

impl<T: PyPayload> Eq for PyInterned<T> {}

impl<T: PyPayload + std::fmt::Debug> std::fmt::Debug for PyInterned<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&**self, f)?;
        write!(f, "@{:p}", self.as_ptr())
    }
}

impl<T: PyPayload> ToPyObject for &'static PyInterned<T> {
    fn to_pyobject(self, _vm: &VirtualMachine) -> PyObjectRef {
        self.to_owned().into()
    }
}

mod sealed {
    use crate::{
        builtins::PyStr,
        object::{Py, PyExact, PyRefExact},
    };

    pub trait SealedInternable {}

    impl SealedInternable for String {}
    impl SealedInternable for &str {}
    impl SealedInternable for PyRefExact<PyStr> {}

    pub trait SealedMaybeInterned {}

    impl SealedMaybeInterned for str {}
    impl SealedMaybeInterned for PyExact<PyStr> {}
    impl SealedMaybeInterned for Py<PyStr> {}
}

/// A sealed marker trait for `DictKey` types that always become an exact instance of `str`
pub trait Internable
where
    Self: sealed::SealedInternable + ToPyObject + AsRef<Self::Interned>,
    Self::Interned: MaybeInterned,
{
    type Interned: ?Sized;
    fn into_pyref_exact(self, str_type: PyTypeRef) -> PyRefExact<PyStr>;
}

impl Internable for String {
    type Interned = str;
    #[inline]
    fn into_pyref_exact(self, str_type: PyTypeRef) -> PyRefExact<PyStr> {
        let obj = PyRef::new_ref(PyStr::from(self), str_type, None);
        unsafe { PyRefExact::new_unchecked(obj) }
    }
}

impl Internable for &str {
    type Interned = str;
    #[inline]
    fn into_pyref_exact(self, str_type: PyTypeRef) -> PyRefExact<PyStr> {
        self.to_owned().into_pyref_exact(str_type)
    }
}

impl Internable for PyRefExact<PyStr> {
    type Interned = Py<PyStr>;
    #[inline]
    fn into_pyref_exact(self, _str_type: PyTypeRef) -> PyRefExact<PyStr> {
        self
    }
}

pub trait MaybeInterned:
    AsRef<str> + crate::dictdatatype::DictKey + sealed::SealedMaybeInterned
{
    fn as_interned(&self) -> Option<&'static PyStrInterned>;
}

impl MaybeInterned for str {
    #[inline(always)]
    fn as_interned(&self) -> Option<&'static PyStrInterned> {
        None
    }
}

impl MaybeInterned for PyExact<PyStr> {
    #[inline(always)]
    fn as_interned(&self) -> Option<&'static PyStrInterned> {
        None
    }
}

impl MaybeInterned for Py<PyStr> {
    #[inline(always)]
    fn as_interned(&self) -> Option<&'static PyStrInterned> {
        if self.as_object().is_interned() {
            Some(unsafe { std::mem::transmute(self) })
        } else {
            None
        }
    }
}

impl PyObject {
    #[inline]
    pub fn as_interned_str(&self, vm: &crate::VirtualMachine) -> Option<&'static PyStrInterned> {
        let s: Option<&Py<PyStr>> = self.downcast_ref();
        if self.is_interned() {
            s.unwrap().as_interned()
        } else if let Some(s) = s {
            vm.ctx.interned_str(s.as_str())
        } else {
            None
        }
    }
}
