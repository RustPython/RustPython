use super::{
    core::{Py, PyObject, PyObjectRef, PyRef},
    payload::{PyObjectPayload, PyPayload},
};
use crate::common::{
    atomic::{Ordering, PyAtomic, Radium},
    lock::PyRwLockReadGuard,
};
use crate::{
    builtins::{PyBaseExceptionRef, PyStrInterned, PyType},
    convert::{IntoPyException, ToPyObject, ToPyResult, TryFromObject},
    VirtualMachine,
};
use std::{borrow::Borrow, fmt, marker::PhantomData, ops::Deref, ptr::null_mut};

/* Python objects and references.

Okay, so each python object itself is an class itself (PyObject). Each
python object can have several references to it (PyObjectRef). These
references are Rc (reference counting) rust smart pointers. So when
all references are destroyed, the object itself also can be cleaned up.
Basically reference counting, but then done by rust.

*/

/*
 * Good reference: https://github.com/ProgVal/pythonvm-rust/blob/master/src/objects/mod.rs
 */

/// Use this type for functions which return a python object or an exception.
/// Both the python object and the python exception are `PyObjectRef` types
/// since exceptions are also python objects.
pub type PyResult<T = PyObjectRef> = Result<T, PyBaseExceptionRef>; // A valid value, or an exception

impl<T: fmt::Display> fmt::Display for PyRef<T>
where
    T: PyObjectPayload + fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(&**self, f)
    }
}
impl<T: fmt::Display> fmt::Display for Py<T>
where
    T: PyObjectPayload + fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(&**self, f)
    }
}

#[repr(transparent)]
pub struct PyExact<T: PyObjectPayload> {
    inner: Py<T>,
}

impl<T: PyPayload> PyExact<T> {
    /// # Safety
    /// Given reference must be exact type of payload T
    #[inline(always)]
    pub unsafe fn ref_unchecked(r: &Py<T>) -> &Self {
        &*(r as *const _ as *const Self)
    }
}

impl<T: PyPayload> Deref for PyExact<T> {
    type Target = Py<T>;
    #[inline(always)]
    fn deref(&self) -> &Py<T> {
        &self.inner
    }
}

impl<T: PyObjectPayload> Borrow<PyObject> for PyExact<T> {
    #[inline(always)]
    fn borrow(&self) -> &PyObject {
        self.inner.borrow()
    }
}

impl<T: PyObjectPayload> AsRef<PyObject> for PyExact<T> {
    #[inline(always)]
    fn as_ref(&self) -> &PyObject {
        self.inner.as_ref()
    }
}

impl<T: PyObjectPayload> Borrow<Py<T>> for PyExact<T> {
    #[inline(always)]
    fn borrow(&self) -> &Py<T> {
        &self.inner
    }
}

impl<T: PyObjectPayload> AsRef<Py<T>> for PyExact<T> {
    #[inline(always)]
    fn as_ref(&self) -> &Py<T> {
        &self.inner
    }
}

impl<T: PyPayload> std::borrow::ToOwned for PyExact<T> {
    type Owned = PyRefExact<T>;
    fn to_owned(&self) -> Self::Owned {
        let owned = self.inner.to_owned();
        unsafe { PyRefExact::new_unchecked(owned) }
    }
}

/// PyRef but guaranteed not to be a subtype instance
#[derive(Debug)]
#[repr(transparent)]
pub struct PyRefExact<T: PyObjectPayload> {
    inner: PyRef<T>,
}

impl<T: PyObjectPayload> PyRefExact<T> {
    /// # Safety
    /// obj must have exact type for the payload
    pub unsafe fn new_unchecked(obj: PyRef<T>) -> Self {
        Self { inner: obj }
    }

    pub fn into_pyref(self) -> PyRef<T> {
        self.inner
    }
}

impl<T: PyObjectPayload> Clone for PyRefExact<T> {
    fn clone(&self) -> Self {
        let inner = self.inner.clone();
        Self { inner }
    }
}

impl<T: PyPayload> TryFromObject for PyRefExact<T> {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        let target_cls = T::class(vm);
        let cls = obj.class();
        if cls.is(target_cls) {
            let obj = obj
                .downcast()
                .map_err(|obj| vm.new_downcast_runtime_error(target_cls, &obj))?;
            Ok(Self { inner: obj })
        } else if cls.fast_issubclass(target_cls) {
            Err(vm.new_type_error(format!(
                "Expected an exact instance of '{}', not a subclass '{}'",
                target_cls.name(),
                cls.name(),
            )))
        } else {
            Err(vm.new_type_error(format!(
                "Expected type '{}', not '{}'",
                target_cls.name(),
                cls.name(),
            )))
        }
    }
}

impl<T: PyPayload> Deref for PyRefExact<T> {
    type Target = PyExact<T>;
    #[inline(always)]
    fn deref(&self) -> &PyExact<T> {
        unsafe { PyExact::ref_unchecked(self.inner.deref()) }
    }
}

impl<T: PyObjectPayload> Borrow<PyObject> for PyRefExact<T> {
    #[inline(always)]
    fn borrow(&self) -> &PyObject {
        self.inner.borrow()
    }
}

impl<T: PyObjectPayload> AsRef<PyObject> for PyRefExact<T> {
    #[inline(always)]
    fn as_ref(&self) -> &PyObject {
        self.inner.as_ref()
    }
}

impl<T: PyObjectPayload> Borrow<Py<T>> for PyRefExact<T> {
    #[inline(always)]
    fn borrow(&self) -> &Py<T> {
        self.inner.borrow()
    }
}

impl<T: PyObjectPayload> AsRef<Py<T>> for PyRefExact<T> {
    #[inline(always)]
    fn as_ref(&self) -> &Py<T> {
        self.inner.as_ref()
    }
}

impl<T: PyPayload> Borrow<PyExact<T>> for PyRefExact<T> {
    #[inline(always)]
    fn borrow(&self) -> &PyExact<T> {
        self
    }
}

impl<T: PyPayload> AsRef<PyExact<T>> for PyRefExact<T> {
    #[inline(always)]
    fn as_ref(&self) -> &PyExact<T> {
        self
    }
}

impl<T: PyPayload> ToPyObject for PyRefExact<T> {
    #[inline(always)]
    fn to_pyobject(self, _vm: &VirtualMachine) -> PyObjectRef {
        self.inner.into()
    }
}

pub struct PyAtomicRef<T> {
    inner: PyAtomic<*mut u8>,
    _phantom: PhantomData<T>,
}

cfg_if::cfg_if! {
    if #[cfg(feature = "threading")] {
        unsafe impl<T: Send + PyObjectPayload> Send for PyAtomicRef<T> {}
        unsafe impl<T: Sync + PyObjectPayload> Sync for PyAtomicRef<T> {}
        unsafe impl<T: Send + PyObjectPayload> Send for PyAtomicRef<Option<T>> {}
        unsafe impl<T: Sync + PyObjectPayload> Sync for PyAtomicRef<Option<T>> {}
        unsafe impl Send for PyAtomicRef<PyObject> {}
        unsafe impl Sync for PyAtomicRef<PyObject> {}
        unsafe impl Send for PyAtomicRef<Option<PyObject>> {}
        unsafe impl Sync for PyAtomicRef<Option<PyObject>> {}
    }
}

impl<T: fmt::Debug> fmt::Debug for PyAtomicRef<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PyAtomicRef(")?;
        unsafe {
            self.inner
                .load(Ordering::Relaxed)
                .cast::<T>()
                .as_ref()
                .fmt(f)
        }?;
        write!(f, ")")
    }
}

impl<T: PyObjectPayload> From<PyRef<T>> for PyAtomicRef<T> {
    fn from(pyref: PyRef<T>) -> Self {
        let py = PyRef::leak(pyref);
        Self {
            inner: Radium::new(py as *const _ as *mut _),
            _phantom: Default::default(),
        }
    }
}

impl<T: PyObjectPayload> Deref for PyAtomicRef<T> {
    type Target = Py<T>;

    fn deref(&self) -> &Self::Target {
        unsafe {
            self.inner
                .load(Ordering::Relaxed)
                .cast::<Py<T>>()
                .as_ref()
                .unwrap_unchecked()
        }
    }
}

impl<T: PyObjectPayload> PyAtomicRef<T> {
    /// # Safety
    /// The caller is responsible to keep the returned PyRef alive
    /// until no more reference can be used via PyAtomicRef::deref()
    #[must_use]
    pub unsafe fn swap(&self, pyref: PyRef<T>) -> PyRef<T> {
        let py = PyRef::leak(pyref) as *const Py<T> as *mut _;
        let old = Radium::swap(&self.inner, py, Ordering::AcqRel);
        PyRef::from_raw(old.cast())
    }

    pub fn swap_to_temporary_refs(&self, pyref: PyRef<T>, vm: &VirtualMachine) {
        let old = unsafe { self.swap(pyref) };
        if let Some(frame) = vm.current_frame() {
            frame.temporary_refs.lock().push(old.into());
        }
    }
}

impl<T: PyObjectPayload> From<Option<PyRef<T>>> for PyAtomicRef<Option<T>> {
    fn from(opt_ref: Option<PyRef<T>>) -> Self {
        let val = opt_ref
            .map(|x| PyRef::leak(x) as *const Py<T> as *mut _)
            .unwrap_or(null_mut());
        Self {
            inner: Radium::new(val),
            _phantom: Default::default(),
        }
    }
}

impl<T: PyObjectPayload> PyAtomicRef<Option<T>> {
    pub fn deref(&self) -> Option<&Py<T>> {
        unsafe { self.inner.load(Ordering::Relaxed).cast::<Py<T>>().as_ref() }
    }

    pub fn to_owned(&self) -> Option<PyRef<T>> {
        self.deref().map(|x| x.to_owned())
    }

    /// # Safety
    /// The caller is responsible to keep the returned PyRef alive
    /// until no more reference can be used via PyAtomicRef::deref()
    #[must_use]
    pub unsafe fn swap(&self, opt_ref: Option<PyRef<T>>) -> Option<PyRef<T>> {
        let val = opt_ref
            .map(|x| PyRef::leak(x) as *const Py<T> as *mut _)
            .unwrap_or(null_mut());
        let old = Radium::swap(&self.inner, val, Ordering::AcqRel);
        unsafe { old.cast::<Py<T>>().as_ref().map(|x| PyRef::from_raw(x)) }
    }

    pub fn swap_to_temporary_refs(&self, opt_ref: Option<PyRef<T>>, vm: &VirtualMachine) {
        let Some(old) = (unsafe { self.swap(opt_ref) }) else {
            return;
        };
        if let Some(frame) = vm.current_frame() {
            frame.temporary_refs.lock().push(old.into());
        }
    }
}

impl From<PyObjectRef> for PyAtomicRef<PyObject> {
    fn from(obj: PyObjectRef) -> Self {
        let obj = obj.into_raw();
        Self {
            inner: Radium::new(obj as *mut _),
            _phantom: Default::default(),
        }
    }
}

impl Deref for PyAtomicRef<PyObject> {
    type Target = PyObject;

    fn deref(&self) -> &Self::Target {
        unsafe {
            self.inner
                .load(Ordering::Relaxed)
                .cast::<PyObject>()
                .as_ref()
                .unwrap_unchecked()
        }
    }
}

impl PyAtomicRef<PyObject> {
    /// # Safety
    /// The caller is responsible to keep the returned PyRef alive
    /// until no more reference can be used via PyAtomicRef::deref()
    #[must_use]
    pub unsafe fn swap(&self, obj: PyObjectRef) -> PyObjectRef {
        let obj = obj.into_raw();
        let old = Radium::swap(&self.inner, obj as *mut _, Ordering::AcqRel);
        PyObjectRef::from_raw(old as _)
    }

    pub fn swap_to_temporary_refs(&self, obj: PyObjectRef, vm: &VirtualMachine) {
        let old = unsafe { self.swap(obj) };
        if let Some(frame) = vm.current_frame() {
            frame.temporary_refs.lock().push(old);
        }
    }
}

impl From<Option<PyObjectRef>> for PyAtomicRef<Option<PyObject>> {
    fn from(obj: Option<PyObjectRef>) -> Self {
        let val = obj.map(|x| x.into_raw() as *mut _).unwrap_or(null_mut());
        Self {
            inner: Radium::new(val),
            _phantom: Default::default(),
        }
    }
}

impl PyAtomicRef<Option<PyObject>> {
    pub fn deref(&self) -> Option<&PyObject> {
        unsafe {
            self.inner
                .load(Ordering::Relaxed)
                .cast::<PyObject>()
                .as_ref()
        }
    }

    pub fn to_owned(&self) -> Option<PyObjectRef> {
        self.deref().map(|x| x.to_owned())
    }

    /// # Safety
    /// The caller is responsible to keep the returned PyRef alive
    /// until no more reference can be used via PyAtomicRef::deref()
    #[must_use]
    pub unsafe fn swap(&self, obj: Option<PyObjectRef>) -> Option<PyObjectRef> {
        let val = obj.map(|x| x.into_raw() as *mut _).unwrap_or(null_mut());
        let old = Radium::swap(&self.inner, val, Ordering::AcqRel);
        old.cast::<PyObject>()
            .as_ref()
            .map(|x| PyObjectRef::from_raw(x))
    }

    pub fn swap_to_temporary_refs(&self, obj: Option<PyObjectRef>, vm: &VirtualMachine) {
        let Some(old) = (unsafe { self.swap(obj) }) else {
            return;
        };
        if let Some(frame) = vm.current_frame() {
            frame.temporary_refs.lock().push(old);
        }
    }
}

pub trait AsObject
where
    Self: Borrow<PyObject>,
{
    #[inline(always)]
    fn as_object(&self) -> &PyObject {
        self.borrow()
    }

    #[inline(always)]
    fn get_id(&self) -> usize {
        self.as_object().unique_id()
    }

    #[inline(always)]
    fn is<T>(&self, other: &T) -> bool
    where
        T: AsObject,
    {
        self.get_id() == other.get_id()
    }

    #[inline(always)]
    fn class(&self) -> &Py<PyType> {
        self.as_object().class()
    }

    fn get_class_attr(&self, attr_name: &'static PyStrInterned) -> Option<PyObjectRef> {
        self.class().get_attr(attr_name)
    }

    /// Determines if `obj` actually an instance of `cls`, this doesn't call __instancecheck__, so only
    /// use this if `cls` is known to have not overridden the base __instancecheck__ magic method.
    #[inline]
    fn fast_isinstance(&self, cls: &Py<PyType>) -> bool {
        self.class().fast_issubclass(cls)
    }
}

impl<T> AsObject for T where T: Borrow<PyObject> {}

impl PyObject {
    #[inline(always)]
    fn unique_id(&self) -> usize {
        self as *const PyObject as usize
    }
}

// impl<T: ?Sized> Borrow<PyObject> for PyRc<T> {
//     #[inline(always)]
//     fn borrow(&self) -> &PyObject {
//         unsafe { &*(&**self as *const T as *const PyObject) }
//     }
// }

/// A borrow of a reference to a Python object. This avoids having clone the `PyRef<T>`/
/// `PyObjectRef`, which isn't that cheap as that increments the atomic reference counter.
pub struct PyLease<'a, T: PyObjectPayload> {
    inner: PyRwLockReadGuard<'a, PyRef<T>>,
}

impl<'a, T: PyObjectPayload + PyPayload> PyLease<'a, T> {
    #[inline(always)]
    pub fn into_owned(self) -> PyRef<T> {
        self.inner.clone()
    }
}

impl<'a, T: PyObjectPayload + PyPayload> Borrow<PyObject> for PyLease<'a, T> {
    #[inline(always)]
    fn borrow(&self) -> &PyObject {
        self.inner.as_ref()
    }
}

impl<'a, T: PyObjectPayload + PyPayload> Deref for PyLease<'a, T> {
    type Target = PyRef<T>;
    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<'a, T> fmt::Display for PyLease<'a, T>
where
    T: PyPayload + fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(&**self, f)
    }
}

impl<T: PyObjectPayload> ToPyObject for PyRef<T> {
    #[inline(always)]
    fn to_pyobject(self, _vm: &VirtualMachine) -> PyObjectRef {
        self.into()
    }
}

impl ToPyObject for PyObjectRef {
    #[inline(always)]
    fn to_pyobject(self, _vm: &VirtualMachine) -> PyObjectRef {
        self
    }
}

impl ToPyObject for &PyObject {
    #[inline(always)]
    fn to_pyobject(self, _vm: &VirtualMachine) -> PyObjectRef {
        self.to_owned()
    }
}

// Allows a built-in function to return any built-in object payload without
// explicitly implementing `ToPyObject`.
impl<T> ToPyObject for T
where
    T: PyPayload + Sized,
{
    #[inline(always)]
    fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        PyPayload::into_pyobject(self, vm)
    }
}

impl<T> ToPyResult for T
where
    T: ToPyObject,
{
    #[inline(always)]
    fn to_pyresult(self, vm: &VirtualMachine) -> PyResult {
        Ok(self.to_pyobject(vm))
    }
}

impl<T, E> ToPyResult for Result<T, E>
where
    T: ToPyObject,
    E: IntoPyException,
{
    #[inline(always)]
    fn to_pyresult(self, vm: &VirtualMachine) -> PyResult {
        self.map(|res| T::to_pyobject(res, vm))
            .map_err(|e| E::into_pyexception(e, vm))
    }
}

impl IntoPyException for PyBaseExceptionRef {
    #[inline(always)]
    fn into_pyexception(self, _vm: &VirtualMachine) -> PyBaseExceptionRef {
        self
    }
}
