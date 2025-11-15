use rustpython_common::wtf8::{Wtf8, Wtf8Buf};

use crate::{
    AsObject, Py, PyExact, PyObject, PyObjectRef, PyPayload, PyRef, PyRefExact, VirtualMachine,
    builtins::{PyStr, PyStrInterned, PyTypeRef},
    common::lock::PyRwLock,
    convert::ToPyObject,
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
    pub unsafe fn intern<S: InternableString>(
        &self,
        s: S,
        typ: PyTypeRef,
    ) -> &'static PyStrInterned {
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
    pub fn interned<S: MaybeInternedString + ?Sized>(
        &self,
        s: &S,
    ) -> Option<&'static PyStrInterned> {
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
        self.inner.as_wtf8().hash(state)
    }
}

impl PartialEq for CachedPyStrRef {
    fn eq(&self, other: &Self) -> bool {
        self.inner.as_wtf8() == other.inner.as_wtf8()
    }
}

impl Eq for CachedPyStrRef {}

impl std::borrow::Borrow<Wtf8> for CachedPyStrRef {
    #[inline]
    fn borrow(&self) -> &Wtf8 {
        self.as_wtf8()
    }
}

impl AsRef<Wtf8> for CachedPyStrRef {
    #[inline]
    fn as_ref(&self) -> &Wtf8 {
        self.as_wtf8()
    }
}

impl CachedPyStrRef {
    /// # Safety
    /// the given cache must be alive while returned reference is alive
    #[inline]
    const unsafe fn as_interned_str(&self) -> &'static PyStrInterned {
        unsafe { std::mem::transmute_copy(self) }
    }

    #[inline]
    fn as_wtf8(&self) -> &Wtf8 {
        self.inner.as_wtf8()
    }
}

pub struct PyInterned<T> {
    inner: Py<T>,
}

impl<T: PyPayload> PyInterned<T> {
    #[inline]
    pub fn leak(cache: PyRef<T>) -> &'static Self {
        unsafe { std::mem::transmute(cache) }
    }

    #[inline]
    const fn as_ptr(&self) -> *const Py<T> {
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

impl<T> AsRef<Py<T>> for PyInterned<T> {
    #[inline(always)]
    fn as_ref(&self) -> &Py<T> {
        &self.inner
    }
}

impl<T> Deref for PyInterned<T> {
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

impl<T: std::fmt::Debug + PyPayload> std::fmt::Debug for PyInterned<T> {
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
    use rustpython_common::wtf8::{Wtf8, Wtf8Buf};

    use crate::{
        builtins::PyStr,
        object::{Py, PyExact, PyRefExact},
    };

    pub trait SealedInternable {}

    impl SealedInternable for String {}
    impl SealedInternable for &str {}
    impl SealedInternable for Wtf8Buf {}
    impl SealedInternable for &Wtf8 {}
    impl SealedInternable for PyRefExact<PyStr> {}

    pub trait SealedMaybeInterned {}

    impl SealedMaybeInterned for str {}
    impl SealedMaybeInterned for Wtf8 {}
    impl SealedMaybeInterned for PyExact<PyStr> {}
    impl SealedMaybeInterned for Py<PyStr> {}
}

/// A sealed marker trait for `DictKey` types that always become an exact instance of `str`
pub trait InternableString: sealed::SealedInternable + ToPyObject + AsRef<Self::Interned> {
    type Interned: MaybeInternedString + ?Sized;
    fn into_pyref_exact(self, str_type: PyTypeRef) -> PyRefExact<PyStr>;
}

impl InternableString for String {
    type Interned = str;
    #[inline]
    fn into_pyref_exact(self, str_type: PyTypeRef) -> PyRefExact<PyStr> {
        let obj = PyRef::new_ref(PyStr::from(self), str_type, None);
        unsafe { PyRefExact::new_unchecked(obj) }
    }
}

impl InternableString for &str {
    type Interned = str;
    #[inline]
    fn into_pyref_exact(self, str_type: PyTypeRef) -> PyRefExact<PyStr> {
        self.to_owned().into_pyref_exact(str_type)
    }
}

impl InternableString for Wtf8Buf {
    type Interned = Wtf8;
    fn into_pyref_exact(self, str_type: PyTypeRef) -> PyRefExact<PyStr> {
        let obj = PyRef::new_ref(PyStr::from(self), str_type, None);
        unsafe { PyRefExact::new_unchecked(obj) }
    }
}

impl InternableString for &Wtf8 {
    type Interned = Wtf8;
    fn into_pyref_exact(self, str_type: PyTypeRef) -> PyRefExact<PyStr> {
        self.to_owned().into_pyref_exact(str_type)
    }
}

impl InternableString for PyRefExact<PyStr> {
    type Interned = Py<PyStr>;
    #[inline]
    fn into_pyref_exact(self, _str_type: PyTypeRef) -> PyRefExact<PyStr> {
        self
    }
}

pub trait MaybeInternedString:
    AsRef<Wtf8> + crate::dict_inner::DictKey + sealed::SealedMaybeInterned
{
    fn as_interned(&self) -> Option<&'static PyStrInterned>;
}

impl MaybeInternedString for str {
    #[inline(always)]
    fn as_interned(&self) -> Option<&'static PyStrInterned> {
        None
    }
}

impl MaybeInternedString for Wtf8 {
    #[inline(always)]
    fn as_interned(&self) -> Option<&'static PyStrInterned> {
        None
    }
}

impl MaybeInternedString for PyExact<PyStr> {
    #[inline(always)]
    fn as_interned(&self) -> Option<&'static PyStrInterned> {
        None
    }
}

impl MaybeInternedString for Py<PyStr> {
    #[inline(always)]
    fn as_interned(&self) -> Option<&'static PyStrInterned> {
        if self.as_object().is_interned() {
            Some(unsafe { std::mem::transmute::<&Self, &PyInterned<PyStr>>(self) })
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
            vm.ctx.interned_str(s.as_wtf8())
        } else {
            None
        }
    }
}
