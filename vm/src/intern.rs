use crate::{
    builtins::{PyStr, PyTypeRef},
    common::lock::PyRwLock,
    Py, PyRef, PyRefExact,
};
use std::ops::Deref;

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
    pub unsafe fn intern<S: Internable>(&self, s: S, typ: PyTypeRef) -> PyRefExact<PyStr> {
        if let Some(found) = self.inner.read().get(s.as_str()) {
            return found.clone().inner;
        }
        let cache = CachedPyStrRef {
            inner: s.into_pyref(typ),
        };
        let inserted = self.inner.write().insert(cache.clone());
        if inserted {
            cache.inner
        } else {
            self.inner
                .read()
                .get(cache.inner.as_str())
                .unwrap()
                .clone()
                .inner
        }
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
    fn borrow(&self) -> &str {
        self.inner.as_str()
    }
}

mod sealed {
    use crate::{builtins::PyStr, object::PyRefExact};

    pub trait SealedInternable {}

    impl SealedInternable for String {}

    impl SealedInternable for &str {}

    impl SealedInternable for PyRefExact<PyStr> {}
}

/// A sealed marker trait for `DictKey` types that always become an exact instance of `str`
pub trait Internable: sealed::SealedInternable + AsRef<Self::Key> {
    type Key: crate::dictdatatype::DictKey + ?Sized;
    fn as_str(&self) -> &str;
    fn into_pyref(self, str_type: PyTypeRef) -> PyRefExact<PyStr>;
}

impl Internable for String {
    type Key = str;
    fn as_str(&self) -> &str {
        String::as_str(self)
    }
    fn into_pyref(self, str_type: PyTypeRef) -> PyRefExact<PyStr> {
        let obj = PyRef::new_ref(PyStr::from(self), str_type, None);
        unsafe { PyRefExact::new_unchecked(obj) }
    }
}

impl Internable for &str {
    type Key = str;
    fn as_str(&self) -> &str {
        self
    }
    fn into_pyref(self, str_type: PyTypeRef) -> PyRefExact<PyStr> {
        self.to_owned().into_pyref(str_type)
    }
}

impl Internable for PyRefExact<PyStr> {
    type Key = Py<PyStr>;
    fn as_str(&self) -> &str {
        self.deref().as_str()
    }
    fn into_pyref(self, _str_type: PyTypeRef) -> PyRefExact<PyStr> {
        self
    }
}
