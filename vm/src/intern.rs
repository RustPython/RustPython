use crate::{
    builtins::{PyStr, PyTypeRef},
    common::lock::PyRwLock,
    convert::ToPyObject,
    Py, PyObject, PyObjectRef, PyRef, PyRefExact,
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
                let interned = unsafe { PyStrInterned::borrow_cache(&cache) };
                // unsafe { interned.as_object().mark_intern() };
                interned
            } else {
                zelf.inner
                    .read()
                    .get(cache.as_str())
                    .map(|cached| unsafe { PyStrInterned::borrow_cache(cached) })
                    .expect("")
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
            .map(|cached| unsafe { PyStrInterned::borrow_cache(cached) })
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
    #[inline]
    fn as_str(&self) -> &str {
        self.inner.as_str()
    }
}

/// The unique reference of interned PyStr
/// Always intended to be used as a static reference
pub struct PyStrInterned {
    inner: Py<PyStr>,
}

impl PyStrInterned {
    /// # Safety
    /// the given cache must be alive while returned reference is alive
    #[inline]
    unsafe fn borrow_cache(cache: &CachedPyStrRef) -> &'static Self {
        std::mem::transmute_copy(cache)
    }

    #[inline]
    fn as_ptr(&self) -> *const Py<PyStr> {
        self as *const _ as *const _
    }

    #[inline]
    pub fn to_owned(&'static self) -> PyRefExact<PyStr> {
        unsafe { (*(&self as *const _ as *const PyRefExact<PyStr>)).clone() }
    }

    #[inline]
    pub fn to_str(&'static self) -> PyRef<PyStr> {
        self.to_owned().into_pyref()
    }

    #[inline]
    pub fn to_object(&'static self) -> PyObjectRef {
        self.to_str().into()
    }
}

impl Borrow<PyObject> for PyStrInterned {
    #[inline(always)]
    fn borrow(&self) -> &PyObject {
        self.inner.borrow()
    }
}

impl Deref for PyStrInterned {
    type Target = Py<PyStr>;
    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl std::hash::Hash for PyStrInterned {
    #[inline(always)]
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::hash::Hash::hash(&(self as *const _), state)
    }
}

impl PartialEq for PyStrInterned {
    #[inline(always)]
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }
}

impl Eq for PyStrInterned {}

impl AsRef<str> for PyStrInterned {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Debug for PyStrInterned {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self.as_str(), f)?;
        write!(f, "@{:p}", self.as_ptr())
    }
}

impl std::fmt::Display for PyStrInterned {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self.as_str(), f)
    }
}

mod sealed {
    use crate::{
        builtins::PyStr,
        object::{Py, PyRefExact},
    };

    pub trait SealedInternable {}

    impl SealedInternable for String {}
    impl SealedInternable for &str {}
    impl SealedInternable for PyRefExact<PyStr> {}

    pub trait SealedMaybeInterned {}

    impl SealedMaybeInterned for str {}
    impl SealedMaybeInterned for PyRefExact<PyStr> {}
    impl SealedMaybeInterned for Py<PyStr> {}
}

/// A sealed marker trait for `DictKey` types that always become an exact instance of `str`
pub trait Internable: sealed::SealedInternable + ToPyObject + AsRef<Self::Interned> {
    type Interned: ?Sized + MaybeInterned;
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

impl MaybeInterned for Py<PyStr> {
    #[inline(always)]
    fn as_interned(&self) -> Option<&'static PyStrInterned> {
        None
    }
}
