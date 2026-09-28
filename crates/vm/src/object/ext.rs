use super::{
    Traverse, TraverseFn,
    core::{Py, PyObject, PyObjectRef, PyRef},
    payload::PyPayload,
};
use crate::common::atomic::{Ordering, PyAtomic, Radium};
use crate::{
    VirtualMachine,
    builtins::{PyBaseExceptionRef, PyStrInterned, PyType},
    convert::{IntoPyException, ToPyObject, ToPyResult, TryFromObject},
    vm::Context,
};
use alloc::fmt;

use core::{
    borrow::Borrow,
    marker::PhantomData,
    ops::Deref,
    ptr::{NonNull, null_mut},
};

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

impl<T: PyPayload + fmt::Display> fmt::Display for PyRef<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&**self, f)
    }
}

impl<T: PyPayload + fmt::Display> fmt::Display for Py<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&**self, f)
    }
}

#[repr(transparent)]
pub struct PyExact<T> {
    inner: Py<T>,
}

impl<T: PyPayload> PyExact<T> {
    /// # Safety
    /// Given reference must be exact type of payload T
    #[inline(always)]
    pub const unsafe fn ref_unchecked(r: &Py<T>) -> &Self {
        unsafe { &*(r as *const _ as *const Self) }
    }
}

impl<T: PyPayload> Deref for PyExact<T> {
    type Target = Py<T>;

    #[inline(always)]
    fn deref(&self) -> &Py<T> {
        &self.inner
    }
}

impl<T: PyPayload> Borrow<PyObject> for PyExact<T> {
    #[inline(always)]
    fn borrow(&self) -> &PyObject {
        self.inner.borrow()
    }
}

impl<T: PyPayload> AsRef<PyObject> for PyExact<T> {
    #[inline(always)]
    fn as_ref(&self) -> &PyObject {
        self.inner.as_ref()
    }
}

impl<T: PyPayload> Borrow<Py<T>> for PyExact<T> {
    #[inline(always)]
    fn borrow(&self) -> &Py<T> {
        &self.inner
    }
}

impl<T: PyPayload> AsRef<Py<T>> for PyExact<T> {
    #[inline(always)]
    fn as_ref(&self) -> &Py<T> {
        &self.inner
    }
}

impl<T: PyPayload> alloc::borrow::ToOwned for PyExact<T> {
    type Owned = PyRefExact<T>;

    fn to_owned(&self) -> Self::Owned {
        let owned = self.inner.to_owned();
        unsafe { PyRefExact::new_unchecked(owned) }
    }
}

impl<T: PyPayload> PyRef<T> {
    pub fn into_exact_or(
        self,
        ctx: &Context,
        f: impl FnOnce(Self) -> PyRefExact<T>,
    ) -> PyRefExact<T> {
        if self.class().is(T::class(ctx)) {
            unsafe { PyRefExact::new_unchecked(self) }
        } else {
            f(self)
        }
    }
}

/// PyRef but guaranteed not to be a subtype instance
#[derive(Debug)]
#[repr(transparent)]
pub struct PyRefExact<T: PyPayload> {
    inner: PyRef<T>,
}

impl<T: PyPayload> PyRefExact<T> {
    /// # Safety
    /// obj must have exact type for the payload
    #[must_use]
    pub const unsafe fn new_unchecked(obj: PyRef<T>) -> Self {
        Self { inner: obj }
    }

    #[must_use]
    pub fn into_pyref(self) -> PyRef<T> {
        self.inner
    }
}

impl<T: PyPayload> Clone for PyRefExact<T> {
    fn clone(&self) -> Self {
        let inner = self.inner.clone();
        Self { inner }
    }
}

impl<T: PyPayload> TryFromObject for PyRefExact<T> {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        let target_cls = T::class(&vm.ctx);
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

impl<T: PyPayload> Borrow<PyObject> for PyRefExact<T> {
    #[inline(always)]
    fn borrow(&self) -> &PyObject {
        self.inner.borrow()
    }
}

impl<T: PyPayload> AsRef<PyObject> for PyRefExact<T> {
    #[inline(always)]
    fn as_ref(&self) -> &PyObject {
        self.inner.as_ref()
    }
}

impl<T: PyPayload> Borrow<Py<T>> for PyRefExact<T> {
    #[inline(always)]
    fn borrow(&self) -> &Py<T> {
        self.inner.borrow()
    }
}

impl<T: PyPayload> AsRef<Py<T>> for PyRefExact<T> {
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

// The cell stores a pointer, not an inline `T`. `PhantomData<T>` would
// otherwise make `PyAtomicRef<PyObject>` `!Unpin` because `PyObject` is pinned.
impl<T> Unpin for PyAtomicRef<T> {}

// Typed and untyped cells are the same pointer-sized slot. A typed nullable
// CAS forwards to the untyped one through this layout.
const _: () = assert!(
    core::mem::size_of::<PyAtomicRef<Option<PyObject>>>()
        == core::mem::size_of::<PyAtomicRef<()>>()
        && core::mem::align_of::<PyAtomicRef<Option<PyObject>>>()
            == core::mem::align_of::<PyAtomicRef<()>>()
        && core::mem::offset_of!(PyAtomicRef<Option<PyObject>>, inner)
            == core::mem::offset_of!(PyAtomicRef<()>, inner)
        && core::mem::offset_of!(PyAtomicRef<Option<PyObject>>, inner) == 0
);

impl<T> Drop for PyAtomicRef<T> {
    fn drop(&mut self) {
        // SAFETY: We are dropping the atomic reference, so we can safely
        // release the pointer.
        unsafe {
            let ptr = Radium::swap(&self.inner, null_mut(), Ordering::Relaxed);
            if let Some(ptr) = NonNull::<PyObject>::new(ptr.cast()) {
                let _: PyObjectRef = PyObjectRef::from_raw(ptr);
            }
        }
    }
}

cfg_select! {
    feature = "threading" => {
        unsafe impl<T: Send + PyPayload> Send for PyAtomicRef<T> {}
        unsafe impl<T: Sync + PyPayload> Sync for PyAtomicRef<T> {}
        unsafe impl<T: Send + PyPayload> Send for PyAtomicRef<Option<T>> {}
        unsafe impl<T: Sync + PyPayload> Sync for PyAtomicRef<Option<T>> {}
        unsafe impl Send for PyAtomicRef<PyObject> {}
        unsafe impl Sync for PyAtomicRef<PyObject> {}
        unsafe impl Send for PyAtomicRef<Option<PyObject>> {}
        unsafe impl Sync for PyAtomicRef<Option<PyObject>> {}
    }
    _ => {}
}

impl<T> fmt::Debug for PyAtomicRef<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PyAtomicRef(")?;
        // The stored pointer is a `Py<T>` — the full object, header included —
        // as `Deref`, `load_raw` and `swap` all read it. Formatting it as a
        // bare payload would skip the header and print misaligned bytes.
        unsafe {
            self.inner
                .load(Ordering::Relaxed)
                .cast::<PyObject>()
                .as_ref()
                .fmt(f)
        }?;
        write!(f, ")")
    }
}

impl<T: PyPayload> From<PyRef<T>> for PyAtomicRef<T> {
    fn from(pyref: PyRef<T>) -> Self {
        let py = PyRef::leak(pyref);
        let ptr = py as *const _ as *mut u8;
        // Expose provenance so we can re-derive via with_exposed_provenance
        // without Stacked Borrows tag restrictions during bootstrap
        ptr.expose_provenance();
        Self {
            inner: Radium::new(ptr),
            _phantom: Default::default(),
        }
    }
}

impl<T: PyPayload> Deref for PyAtomicRef<T> {
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

impl<T: PyPayload> PyAtomicRef<T> {
    /// Move a reference into an atomic pointer without creating a Rust
    /// reference to the pointee. This is only for bootstrap objects whose
    /// allocation is valid but whose payload is still being initialized.
    ///
    /// # Safety
    /// The pointee must remain allocated, and this atomic reference must not
    /// be dereferenced until the pointee has been fully initialized.
    pub(super) unsafe fn from_ref_without_retag(pyref: PyRef<T>) -> Self {
        let ptr = pyref.into_non_null().as_ptr().cast::<u8>();
        ptr.expose_provenance();
        Self {
            inner: Radium::new(ptr),
            _phantom: Default::default(),
        }
    }

    /// Load the raw pointer without creating a reference.
    /// Avoids Stacked Borrows retag, safe for use during bootstrap
    /// when type objects have self-referential pointers being mutated.
    #[inline(always)]
    pub(super) fn load_raw(&self) -> *const Py<T> {
        self.inner.load(Ordering::Relaxed).cast::<Py<T>>()
    }

    /// # Safety
    /// The caller is responsible to keep the returned PyRef alive
    /// until no more reference can be used via PyAtomicRef::deref()
    #[must_use]
    pub unsafe fn swap(&self, pyref: PyRef<T>) -> PyRef<T> {
        let py = PyRef::leak(pyref) as *const Py<T> as *mut _;
        let old = Radium::swap(&self.inner, py, Ordering::AcqRel);
        unsafe { PyRef::from_raw(old.cast()) }
    }

    pub fn swap_to_temporary_refs(&self, pyref: PyRef<T>, vm: &VirtualMachine) {
        let old = unsafe { self.swap(pyref) };
        if let Some(frame) = vm.current_frame() {
            frame.iframe().cold().temporary_refs.lock().push(old.into());
        }
    }

    /// Strong reference to the current value.
    ///
    /// The cell is never null. A concurrent store may drop the previous value;
    /// the incref is retried until it applies to the pointer still in the slot.
    /// A null load is retried rather than forged.
    pub(crate) fn load_owned(&self) -> PyRef<T> {
        loop {
            if let Some(obj) = cell_load_owned(&self.inner) {
                // SAFETY: this cell is only stored with `PyRef<T>`.
                return unsafe { obj.downcast_unchecked() };
            }
            core::hint::spin_loop();
        }
    }

    /// Replace the stored reference. Returns the previous one, still owned.
    ///
    /// `None` only if the cell was empty. Callers that publish through
    /// `From<PyRef<T>>` and `store` keep a `T` in the slot.
    pub(crate) fn store(&self, value: PyRef<T>) -> Option<PyRef<T>> {
        cell_store(&self.inner, Some(value.into())).map(|obj| {
            // SAFETY: this cell is only stored with `PyRef<T>`.
            unsafe { obj.downcast_unchecked() }
        })
    }
}

impl<T: PyPayload> From<Option<PyRef<T>>> for PyAtomicRef<Option<T>> {
    fn from(opt_ref: Option<PyRef<T>>) -> Self {
        let val = opt_ref.map_or(null_mut(), |x| PyRef::leak(x) as *const Py<T> as *mut _);
        Self {
            inner: Radium::new(val),
            _phantom: Default::default(),
        }
    }
}

impl<T: PyPayload> PyAtomicRef<Option<T>> {
    /// Optional form of PyAtomicRef::from_ref_without_retag.
    ///
    /// # Safety
    /// A non-None pointee must remain allocated, and this atomic reference
    /// must not be dereferenced until the pointee has been fully initialized.
    pub(super) unsafe fn from_optional_ref_without_retag(opt_ref: Option<PyRef<T>>) -> Self {
        let ptr = opt_ref.map_or(null_mut(), |pyref| {
            pyref.into_non_null().as_ptr().cast::<u8>()
        });
        ptr.expose_provenance();
        Self {
            inner: Radium::new(ptr),
            _phantom: Default::default(),
        }
    }

    pub fn deref(&self) -> Option<&Py<T>> {
        self.deref_ordering(Ordering::Relaxed)
    }

    pub fn deref_ordering(&self, ordering: Ordering) -> Option<&Py<T>> {
        unsafe { self.inner.load(ordering).cast::<Py<T>>().as_ref() }
    }

    /// # Safety
    /// The caller is responsible to keep the returned PyRef alive
    /// until no more reference can be used via PyAtomicRef::deref()
    #[must_use]
    pub unsafe fn swap(&self, opt_ref: Option<PyRef<T>>) -> Option<PyRef<T>> {
        let val = opt_ref.map_or(null_mut(), |x| PyRef::leak(x) as *const Py<T> as *mut _);
        let old = Radium::swap(&self.inner, val, Ordering::AcqRel);
        unsafe { old.cast::<Py<T>>().as_ref().map(|x| PyRef::from_raw(x)) }
    }

    pub fn swap_to_temporary_refs(&self, opt_ref: Option<PyRef<T>>, vm: &VirtualMachine) {
        let Some(old) = (unsafe { self.swap(opt_ref) }) else {
            return;
        };
        if let Some(frame) = vm.current_frame() {
            frame.iframe().cold().temporary_refs.lock().push(old.into());
        }
    }

    /// Strong reference to the current value, or `None` when the slot is empty.
    ///
    /// This is the owned read for a nullable cell. A concurrent store may drop
    /// the previous value; the incref is retried until it applies to the
    /// pointer still in the slot. Published-object memory is reclaimed only
    /// after a QSBR grace period (see `object::qsbr`), so the refcount word of
    /// a swapped-out value stays readable.
    pub fn load_owned(&self) -> Option<PyRef<T>> {
        cell_load_owned(&self.inner).map(|obj| {
            // SAFETY: a typed cell only stores references of payload `T`.
            unsafe { obj.downcast_unchecked() }
        })
    }

    /// Replace the stored reference. Returns the previous one, still owned.
    pub(crate) fn store(&self, value: Option<PyRef<T>>) -> Option<PyRef<T>> {
        cell_store(&self.inner, value.map(PyObjectRef::from)).map(|obj| {
            // SAFETY: a typed cell only stores references of payload `T`.
            unsafe { obj.downcast_unchecked() }
        })
    }

    /// Store `value` only when the cell is empty.
    ///
    /// On failure the cell is unchanged and `value` is returned still owned.
    pub(crate) fn compare_exchange_empty(&self, value: PyRef<T>) -> Result<(), PyRef<T>> {
        // SAFETY: every `PyAtomicRef<_>` is an atomic pointer plus a
        // zero-sized marker, so the layouts match. The untyped cell API only
        // loads and stores that pointer.
        let cell = unsafe { &*core::ptr::from_ref(self).cast::<PyAtomicRef<Option<PyObject>>>() };
        cell.compare_exchange_empty(value.into()).map_err(|obj| {
            // SAFETY: a typed cell only stores references of payload `T`.
            unsafe { obj.downcast_unchecked() }
        })
    }
}

fn cell_load_ptr(inner: &PyAtomic<*mut u8>) -> *mut PyObject {
    inner.load(Ordering::Acquire).cast()
}

/// Try-incref the pointer in `inner`.
///
/// A concurrent store may drop the previous value. The incref is retried
/// until it applies to the pointer still in the slot. Returns `None` when
/// the slot is empty.
fn cell_load_owned(inner: &PyAtomic<*mut u8>) -> Option<PyObjectRef> {
    let ptr = inner.load(Ordering::Acquire);
    if ptr.is_null() {
        return None;
    }
    // Without threading the slot's own reference keeps the object alive,
    // so one incref is enough. With threading, retry when a store retires
    // the pointer between the load and the incref.
    #[cfg(not(feature = "threading"))]
    {
        // SAFETY: `ptr` is non-null and the cell's own reference keeps the
        // object alive for this incref.
        unsafe { PyObject::try_to_owned_from_ptr(ptr.cast()) }
    }
    #[cfg(feature = "threading")]
    {
        let mut ptr = ptr;
        loop {
            // SAFETY: `ptr` is non-null. A value that left the cell was marked
            // published, so its refcount word stays readable until QSBR.
            if let Some(obj) = unsafe { PyObject::try_to_owned_from_ptr(ptr.cast()) }
                && core::ptr::eq(inner.load(Ordering::Acquire), ptr)
            {
                return Some(obj);
            }
            ptr = inner.load(Ordering::Acquire);
            if ptr.is_null() {
                return None;
            }
            core::hint::spin_loop();
        }
    }
}

/// Replace the stored reference. Returns the previous one, still owned.
///
/// The value placed in the slot is not marked published, so it can still
/// return to the freelist. With threading, the value that leaves the slot
/// is marked so its free waits out a reader that already loaded it.
fn cell_store(inner: &PyAtomic<*mut u8>, value: Option<PyObjectRef>) -> Option<PyObjectRef> {
    let new_ptr = match value {
        Some(obj) => {
            let ptr = obj.into_raw().as_ptr();
            ptr.expose_provenance();
            ptr.cast()
        }
        None => null_mut(),
    };
    let old = Radium::swap(inner, new_ptr, Ordering::AcqRel);
    // SAFETY: a non-null slot pointer is an owning reference the cell just released.
    let old = NonNull::new(old.cast()).map(|ptr| unsafe { PyObjectRef::from_raw(ptr) });
    #[cfg(feature = "threading")]
    if let Some(old) = old.as_ref() {
        old.mark_cache_published();
    }
    old
}

/// Store `value` only when the cell is empty.
///
/// On failure the cell is unchanged and `value` is returned still owned.
fn cell_compare_exchange_empty(
    inner: &PyAtomic<*mut u8>,
    value: PyObjectRef,
) -> Result<(), PyObjectRef> {
    let raw = value.into_raw();
    let ptr = raw.as_ptr();
    ptr.expose_provenance();
    match inner.compare_exchange(
        core::ptr::null_mut(),
        ptr.cast(),
        Ordering::AcqRel,
        Ordering::Acquire,
    ) {
        Ok(_) => Ok(()),
        Err(_) => {
            // SAFETY: the exchange did not take the pointer, so `raw` is
            // still the unique owning reference.
            Err(unsafe { PyObjectRef::from_raw(raw) })
        }
    }
}

impl From<PyObjectRef> for PyAtomicRef<PyObject> {
    fn from(obj: PyObjectRef) -> Self {
        let obj = obj.into_raw();
        Self {
            inner: Radium::new(obj.cast().as_ptr()),
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
    /// Strong reference to the current value.
    ///
    /// The cell is never null. A concurrent store may drop the previous value;
    /// the incref is retried until it applies to the pointer still in the slot.
    pub(crate) fn load_owned(&self) -> PyObjectRef {
        cell_load_owned(&self.inner).expect("non-null atomic cell")
    }

    /// Replace the stored reference. Returns the previous one, still owned.
    ///
    /// The cell is never null, before and after the store.
    pub(crate) fn store(&self, value: PyObjectRef) -> PyObjectRef {
        cell_store(&self.inner, Some(value)).expect("non-null atomic cell")
    }

    /// # Safety
    /// The caller is responsible to keep the returned PyRef alive
    /// until no more reference can be used via PyAtomicRef::deref()
    #[must_use]
    pub unsafe fn swap(&self, obj: PyObjectRef) -> PyObjectRef {
        let obj = obj.into_raw();
        let old = Radium::swap(&self.inner, obj.cast().as_ptr(), Ordering::AcqRel);
        unsafe { PyObjectRef::from_raw(NonNull::new_unchecked(old.cast())) }
    }

    pub fn swap_to_temporary_refs(&self, obj: PyObjectRef, vm: &VirtualMachine) {
        let old = unsafe { self.swap(obj) };
        if let Some(frame) = vm.current_frame() {
            frame.iframe().cold().temporary_refs.lock().push(old);
        }
    }
}

impl From<Option<PyObjectRef>> for PyAtomicRef<Option<PyObject>> {
    fn from(obj: Option<PyObjectRef>) -> Self {
        let val = obj.map_or(null_mut(), |x| x.into_raw().as_ptr().cast());
        Self {
            inner: Radium::new(val),
            _phantom: Default::default(),
        }
    }
}

impl PyAtomicRef<Option<PyObject>> {
    /// Empty slot. The pointer is null and owns no reference.
    pub(crate) const fn new_empty() -> Self {
        Self {
            inner: {
                #[cfg(feature = "threading")]
                {
                    core::sync::atomic::AtomicPtr::new(null_mut())
                }
                #[cfg(not(feature = "threading"))]
                {
                    core::cell::Cell::new(null_mut())
                }
            },
            _phantom: PhantomData,
        }
    }

    /// Borrowed pointer currently stored. Null when the slot is empty.
    ///
    /// The slot owns the reference, so this stays valid while the slot is
    /// unchanged. Traversal calls it with other threads stopped.
    pub(crate) fn load_ptr(&self) -> *mut PyObject {
        cell_load_ptr(&self.inner)
    }

    /// Strong reference to the current value, or `None` when the slot is empty.
    ///
    /// This is the owned read for a nullable cell. A concurrent store may drop
    /// the previous value; the incref is retried until it applies to the
    /// pointer still in the slot. Published-object memory is reclaimed only
    /// after a QSBR grace period (see `object::qsbr`), so the refcount word of
    /// a swapped-out value stays readable.
    pub fn load_owned(&self) -> Option<PyObjectRef> {
        cell_load_owned(&self.inner)
    }

    /// Replace the stored reference. Returns the previous one, still owned.
    pub(crate) fn store(&self, value: Option<PyObjectRef>) -> Option<PyObjectRef> {
        cell_store(&self.inner, value)
    }

    /// Store `value` only when the cell is empty.
    ///
    /// On failure the cell is unchanged and `value` is returned still owned.
    pub(crate) fn compare_exchange_empty(&self, value: PyObjectRef) -> Result<(), PyObjectRef> {
        cell_compare_exchange_empty(&self.inner, value)
    }

    pub fn deref(&self) -> Option<&PyObject> {
        self.deref_ordering(Ordering::Relaxed)
    }

    pub fn deref_ordering(&self, ordering: Ordering) -> Option<&PyObject> {
        unsafe { self.inner.load(ordering).cast::<PyObject>().as_ref() }
    }

    /// # Safety
    /// The caller is responsible to keep the returned PyRef alive
    /// until no more reference can be used via PyAtomicRef::deref()
    #[must_use]
    pub unsafe fn swap(&self, obj: Option<PyObjectRef>) -> Option<PyObjectRef> {
        let val = obj.map_or(null_mut(), |x| x.into_raw().as_ptr().cast());
        let old = Radium::swap(&self.inner, val, Ordering::AcqRel);
        unsafe { NonNull::new(old.cast::<PyObject>()).map(|x| PyObjectRef::from_raw(x)) }
    }

    pub fn swap_to_temporary_refs(&self, obj: Option<PyObjectRef>, vm: &VirtualMachine) {
        let Some(old) = (unsafe { self.swap(obj) }) else {
            return;
        };
        if let Some(frame) = vm.current_frame() {
            frame.iframe().cold().temporary_refs.lock().push(old);
        }
    }
}

/// A nullable object reference that can be replaced without invalidating readers.
///
/// Unlike [`PyAtomicRef`], this cell only exposes owned reads. Concurrent stores
/// use the same QSBR reclamation as object slots without retaining old values
/// for the lifetime of a Python frame.
#[repr(transparent)]
pub struct PyObjectCell(PyAtomicRef<Option<PyObject>>);

impl From<Option<PyObjectRef>> for PyObjectCell {
    fn from(value: Option<PyObjectRef>) -> Self {
        Self(value.into())
    }
}

impl PyObjectCell {
    /// Return an owned reference to the current value, or `None` for an empty cell.
    #[inline]
    pub fn load_owned(&self) -> Option<PyObjectRef> {
        self.0.load_owned()
    }

    /// Replace the stored value and return the previous reference, still owned.
    #[inline]
    pub fn store(&self, value: Option<PyObjectRef>) -> Option<PyObjectRef> {
        self.0.store(value)
    }
}

impl fmt::Debug for PyObjectCell {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("PyObjectCell")
            .field(&self.load_owned())
            .finish()
    }
}

// SAFETY: the underlying cell visits its single owned reference while mutating
// threads are stopped. It does not clone the reference during traversal.
unsafe impl Traverse for PyObjectCell {
    #[inline]
    fn traverse(&self, traverse_fn: &mut TraverseFn<'_>) {
        self.0.traverse(traverse_fn);
    }
}

// Object members address a single pointer-sized cell at the field's offset.
const _: () = assert!(
    core::mem::size_of::<PyObjectCell>() == core::mem::size_of::<*mut PyObject>()
        && core::mem::align_of::<PyObjectCell>() == core::mem::align_of::<*mut PyObject>()
);

/// Atomic borrowed (non-ref-counted) optional reference to a Python object.
/// Unlike `PyAtomicRef`, this does NOT own the reference.
/// The pointed-to object must outlive this reference.
pub struct PyAtomicBorrow {
    inner: PyAtomic<*mut u8>,
}

// Safety: Access patterns ensure the pointed-to object outlives this reference.
// The owner (generator/coroutine) clears this in its Drop impl before deallocation.
unsafe impl Send for PyAtomicBorrow {}
unsafe impl Sync for PyAtomicBorrow {}

impl PyAtomicBorrow {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Radium::new(null_mut()),
        }
    }

    pub fn store(&self, obj: &PyObject) {
        let ptr = obj as *const PyObject as *mut u8;
        Radium::store(&self.inner, ptr, Ordering::Relaxed);
    }

    pub fn load(&self) -> Option<&PyObject> {
        let ptr = Radium::load(&self.inner, Ordering::Relaxed);
        if ptr.is_null() {
            None
        } else {
            Some(unsafe { &*(ptr as *const PyObject) })
        }
    }

    pub fn clear(&self) {
        Radium::store(&self.inner, null_mut(), Ordering::Relaxed);
    }

    pub fn to_owned(&self) -> Option<PyObjectRef> {
        self.load().map(|obj| obj.to_owned())
    }
}

impl Default for PyAtomicBorrow {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for PyAtomicBorrow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "PyAtomicBorrow({:?})",
            Radium::load(&self.inner, Ordering::Relaxed)
        )
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
        self as *const Self as usize
    }
}

// impl<T: ?Sized> Borrow<PyObject> for PyRc<T> {
//     #[inline(always)]
//     fn borrow(&self) -> &PyObject {
//         unsafe { &*(&**self as *const T as *const PyObject) }
//     }
// }

impl<T: PyPayload> ToPyObject for PyRef<T> {
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
    T: PyPayload + core::fmt::Debug + Sized,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_cell_snapshots_survive_replacement_and_clear() {
        crate::Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let cell = PyObjectCell::from(Some(vm.ctx.new_bytes(vec![1, 2, 3]).into()));
            let first = cell.load_owned().unwrap();
            let previous = cell.store(Some(vm.ctx.new_bytes(vec![4, 5, 6]).into()));
            assert!(previous.as_ref().unwrap().is(&first));
            drop(previous);
            assert_eq!(first.strong_count(), 1);
            assert_eq!(
                first
                    .downcast_ref::<crate::builtins::PyBytes>()
                    .unwrap()
                    .as_bytes(),
                &[1, 2, 3]
            );

            let second = cell.load_owned().unwrap();
            let previous = cell.store(None);
            assert!(previous.as_ref().unwrap().is(&second));
            drop(previous);
            assert!(cell.load_owned().is_none());
            assert!(cell.store(None).is_none());
            assert_eq!(second.strong_count(), 1);
            assert_eq!(
                second
                    .downcast_ref::<crate::builtins::PyBytes>()
                    .unwrap()
                    .as_bytes(),
                &[4, 5, 6]
            );
        });
    }

    #[test]
    fn object_cell_traverses_current_reference_once_without_cloning() {
        crate::Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let cell = PyObjectCell::from(None);
            cell.traverse(&mut |_| panic!("empty cell owns no edge"));
            let value: PyObjectRef = vm.ctx.new_bytes(vec![1, 2, 3]).into();
            assert!(cell.store(Some(value.clone())).is_none());
            let references = value.strong_count();
            let mut edges = 0;
            cell.traverse(&mut |child| {
                assert!(child.is(&value));
                assert_eq!(value.strong_count(), references);
                edges += 1;
            });
            assert_eq!(edges, 1);
            drop(cell.store(None));
            cell.traverse(&mut |_| panic!("cleared cell owns no edge"));
        });
    }
}
