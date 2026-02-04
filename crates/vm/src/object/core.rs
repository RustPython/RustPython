//! Essential types for object models
//!
//! +-------------------------+--------------+-----------------------+
//! |       Management        |       Typed      |      Untyped      |
//! +-------------------------+------------------+-------------------+
//! | Interpreter-independent | [`Py<T>`]        | [`PyObject`]      |
//! | Reference-counted       | [`PyRef<T>`]     | [`PyObjectRef`]   |
//! | Weak                    | [`PyWeakRef<T>`] | [`PyRef<PyWeak>`] |
//! +-------------------------+--------------+-----------------------+
//!
//! [`PyRef<PyWeak>`] may looking like to be called as PyObjectWeak by the rule,
//! but not to do to remember it is a PyRef object.
use super::{
    PyAtomicRef,
    ext::{AsObject, PyRefExact, PyResult},
    payload::PyPayload,
};
use crate::object::traverse_object::PyObjVTable;
use crate::{
    builtins::{PyDictRef, PyType, PyTypeRef},
    common::{
        atomic::{Ordering, PyAtomic, Radium},
        linked_list::{Link, Pointers},
        lock::PyRwLock,
        refcount::RefCount,
    },
    vm::VirtualMachine,
};
use crate::{
    class::StaticType,
    object::traverse::{MaybeTraverse, Traverse, TraverseFn},
};
use itertools::Itertools;

use alloc::fmt;

use core::{
    any::TypeId,
    borrow::Borrow,
    cell::UnsafeCell,
    marker::PhantomData,
    mem::ManuallyDrop,
    ops::Deref,
    ptr::{self, NonNull},
};

// so, PyObjectRef is basically equivalent to `PyRc<PyInner<dyn PyObjectPayload>>`, except it's
// only one pointer in width rather than 2. We do that by manually creating a vtable, and putting
// a &'static reference to it inside the `PyRc` rather than adjacent to it, like trait objects do.
// This can lead to faster code since there's just less data to pass around, as well as because of
// some weird stuff with trait objects, alignment, and padding.
//
// So, every type has an alignment, which means that if you create a value of it it's location in
// memory has to be a multiple of it's alignment. e.g., a type with alignment 4 (like i32) could be
// at 0xb7befbc0, 0xb7befbc4, or 0xb7befbc8, but not 0xb7befbc2. If you have a struct and there are
// 2 fields whose sizes/alignments don't perfectly fit in with each other, e.g.:
// +-------------+-------------+---------------------------+
// |     u16     |      ?      |            i32            |
// | 0x00 | 0x01 | 0x02 | 0x03 | 0x04 | 0x05 | 0x06 | 0x07 |
// +-------------+-------------+---------------------------+
// There has to be padding in the space between the 2 fields. But, if that field is a trait object
// (like `dyn PyObjectPayload`) we don't *know* how much padding there is between the `payload`
// field and the previous field. So, Rust has to consult the vtable to know the exact offset of
// `payload` in `PyInner<dyn PyObjectPayload>`, which has a huge performance impact when *every
// single payload access* requires a vtable lookup. Thankfully, we're able to avoid that because of
// the way we use PyObjectRef, in that whenever we want to access the payload we (almost) always
// access it from a generic function. So, rather than doing
//
// - check vtable for payload offset
// - get offset in PyInner struct
// - call as_any() method of PyObjectPayload
// - call downcast_ref() method of Any
// we can just do
// - check vtable that typeid matches
// - pointer cast directly to *const PyInner<T>
//
// and at that point the compiler can know the offset of `payload` for us because **we've given it a
// concrete type to work with before we ever access the `payload` field**

/// A type to just represent "we've erased the type of this object, cast it before you use it"
#[derive(Debug)]
pub(super) struct Erased;

/// Default dealloc: handles __del__, weakref clearing, tp_clear, and memory free.
/// Equivalent to subtype_dealloc.
pub(super) unsafe fn default_dealloc<T: PyPayload>(obj: *mut PyObject) {
    let obj_ref = unsafe { &*(obj as *const PyObject) };
    if let Err(()) = obj_ref.drop_slow_inner() {
        return; // resurrected by __del__
    }

    let vtable = obj_ref.0.vtable;

    // Untrack from GC BEFORE deallocation.
    if obj_ref.is_gc_tracked() {
        let ptr = unsafe { NonNull::new_unchecked(obj) };
        rustpython_common::refcount::try_defer_drop(move || {
            // untrack_object only removes the pointer address from a HashSet.
            // It does NOT dereference the pointer, so it's safe even after deallocation.
            unsafe {
                crate::gc_state::gc_state().untrack_object(ptr);
            }
        });
    }

    // Extract child references before deallocation to break circular refs (tp_clear)
    let mut edges = Vec::new();
    if let Some(clear_fn) = vtable.clear {
        unsafe { clear_fn(obj, &mut edges) };
    }

    // Deallocate the object memory
    drop(unsafe { Box::from_raw(obj as *mut PyInner<T>) });

    // Drop child references - may trigger recursive destruction.
    // The object is already deallocated, so circular refs are broken.
    drop(edges);
}
pub(super) unsafe fn debug_obj<T: PyPayload + core::fmt::Debug>(
    x: &PyObject,
    f: &mut fmt::Formatter<'_>,
) -> fmt::Result {
    let x = unsafe { &*(x as *const PyObject as *const PyInner<T>) };
    fmt::Debug::fmt(x, f)
}

/// Call `try_trace` on payload
pub(super) unsafe fn try_traverse_obj<T: PyPayload>(x: &PyObject, tracer_fn: &mut TraverseFn<'_>) {
    let x = unsafe { &*(x as *const PyObject as *const PyInner<T>) };
    let payload = &x.payload;
    payload.try_traverse(tracer_fn)
}

/// Call `try_clear` on payload to extract child references (tp_clear)
pub(super) unsafe fn try_clear_obj<T: PyPayload>(x: *mut PyObject, out: &mut Vec<PyObjectRef>) {
    let x = unsafe { &mut *(x as *mut PyInner<T>) };
    x.payload.try_clear(out);
}

bitflags::bitflags! {
    /// GC bits for free-threading support (like ob_gc_bits in Py_GIL_DISABLED)
    /// These bits are stored in a separate atomic field for lock-free access.
    /// See Include/internal/pycore_gc.h
    #[derive(Copy, Clone, Debug, Default)]
    pub(crate) struct GcBits: u8 {
        /// Tracked by the GC
        const TRACKED = 1 << 0;
        /// tp_finalize was called (prevents __del__ from being called twice)
        const FINALIZED = 1 << 1;
        /// Object is unreachable (during GC collection)
        const UNREACHABLE = 1 << 2;
        /// Object is frozen (immutable)
        const FROZEN = 1 << 3;
        /// Memory the object references is shared between multiple threads
        /// and needs special handling when freeing due to possible in-flight lock-free reads
        const SHARED = 1 << 4;
        /// Memory of the object itself is shared between multiple threads
        /// Objects with this bit that are GC objects will automatically be delay-freed
        const SHARED_INLINE = 1 << 5;
        /// Use deferred reference counting
        const DEFERRED = 1 << 6;
    }
}

/// This is an actual python object. It consists of a `typ` which is the
/// python class, and carries some rust payload optionally. This rust
/// payload can be a rust float or rust int in case of float and int objects.
#[repr(C)]
pub(super) struct PyInner<T> {
    pub(super) ref_count: RefCount,
    pub(super) vtable: &'static PyObjVTable,
    /// GC bits for free-threading (like ob_gc_bits)
    pub(super) gc_bits: PyAtomic<u8>,

    pub(super) typ: PyAtomicRef<PyType>, // __class__ member
    pub(super) dict: Option<InstanceDict>,
    pub(super) weak_list: WeakRefList,
    pub(super) slots: Box<[PyRwLock<Option<PyObjectRef>>]>,

    pub(super) payload: T,
}
pub(crate) const SIZEOF_PYOBJECT_HEAD: usize = core::mem::size_of::<PyInner<()>>();

impl<T: fmt::Debug> fmt::Debug for PyInner<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[PyObject {:?}]", &self.payload)
    }
}

unsafe impl<T: MaybeTraverse> Traverse for Py<T> {
    /// DO notice that call `trace` on `Py<T>` means apply `tracer_fn` on `Py<T>`'s children,
    /// not like call `trace` on `PyRef<T>` which apply `tracer_fn` on `PyRef<T>` itself
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        self.0.traverse(tracer_fn)
    }
}

unsafe impl Traverse for PyObject {
    /// DO notice that call `trace` on `PyObject` means apply `tracer_fn` on `PyObject`'s children,
    /// not like call `trace` on `PyObjectRef` which apply `tracer_fn` on `PyObjectRef` itself
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        self.0.traverse(tracer_fn)
    }
}

// === Stripe lock for weakref list protection (WEAKREF_LIST_LOCK) ===

#[cfg(feature = "threading")]
mod weakref_lock {
    use core::sync::atomic::{AtomicU8, Ordering};

    const NUM_WEAKREF_LOCKS: usize = 64;

    static LOCKS: [AtomicU8; NUM_WEAKREF_LOCKS] = [const { AtomicU8::new(0) }; NUM_WEAKREF_LOCKS];

    pub(super) struct WeakrefLockGuard {
        idx: usize,
    }

    impl Drop for WeakrefLockGuard {
        fn drop(&mut self) {
            LOCKS[self.idx].store(0, Ordering::Release);
        }
    }

    pub(super) fn lock(addr: usize) -> WeakrefLockGuard {
        let idx = (addr >> 4) % NUM_WEAKREF_LOCKS;
        loop {
            if LOCKS[idx]
                .compare_exchange_weak(0, 1, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                return WeakrefLockGuard { idx };
            }
            core::hint::spin_loop();
        }
    }

    /// Reset all weakref stripe locks after fork in child process.
    /// Locks held by parent threads would cause infinite spin in the child.
    #[cfg(unix)]
    pub(crate) fn reset_all_after_fork() {
        for lock in &LOCKS {
            lock.store(0, Ordering::Release);
        }
    }
}

#[cfg(not(feature = "threading"))]
mod weakref_lock {
    pub(super) struct WeakrefLockGuard;

    impl Drop for WeakrefLockGuard {
        fn drop(&mut self) {}
    }

    pub(super) fn lock(_addr: usize) -> WeakrefLockGuard {
        WeakrefLockGuard
    }
}

/// Reset weakref stripe locks after fork. Must be called before any
/// Python code runs in the child process.
#[cfg(all(unix, feature = "threading"))]
pub(crate) fn reset_weakref_locks_after_fork() {
    weakref_lock::reset_all_after_fork();
}

// === WeakRefList: inline on every object (tp_weaklist) ===

pub(super) struct WeakRefList {
    /// Head of the intrusive doubly-linked list of weakrefs.
    head: PyAtomic<*mut Py<PyWeak>>,
    /// Cached generic weakref (no callback, exact weakref type).
    /// Matches try_reuse_basic_ref in weakrefobject.c.
    generic: PyAtomic<*mut Py<PyWeak>>,
}

impl fmt::Debug for WeakRefList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WeakRefList").finish_non_exhaustive()
    }
}

/// Unlink a node from the weakref list. Must be called under stripe lock.
///
/// # Safety
/// `node` must be a valid pointer to a node currently in the list owned by `wrl`.
unsafe fn unlink_weakref(wrl: &WeakRefList, node: NonNull<Py<PyWeak>>) {
    unsafe {
        let mut ptrs = WeakLink::pointers(node);
        let prev = ptrs.as_ref().get_prev();
        let next = ptrs.as_ref().get_next();

        if let Some(prev) = prev {
            WeakLink::pointers(prev).as_mut().set_next(next);
        } else {
            // node is the head
            wrl.head.store(
                next.map_or(ptr::null_mut(), |p| p.as_ptr()),
                Ordering::Relaxed,
            );
        }
        if let Some(next) = next {
            WeakLink::pointers(next).as_mut().set_prev(prev);
        }

        ptrs.as_mut().set_prev(None);
        ptrs.as_mut().set_next(None);
    }
}

impl WeakRefList {
    pub fn new() -> Self {
        Self {
            head: Radium::new(ptr::null_mut()),
            generic: Radium::new(ptr::null_mut()),
        }
    }

    /// get_or_create_weakref
    fn add(
        &self,
        obj: &PyObject,
        cls: PyTypeRef,
        cls_is_weakref: bool,
        callback: Option<PyObjectRef>,
        dict: Option<PyDictRef>,
    ) -> PyRef<PyWeak> {
        let is_generic = cls_is_weakref && callback.is_none();
        let _lock = weakref_lock::lock(obj as *const PyObject as usize);

        // try_reuse_basic_ref: reuse cached generic weakref
        if is_generic {
            let generic_ptr = self.generic.load(Ordering::Relaxed);
            if !generic_ptr.is_null() {
                let generic = unsafe { &*generic_ptr };
                if generic.0.ref_count.safe_inc() {
                    return unsafe { PyRef::from_raw(generic_ptr) };
                }
            }
        }

        // Allocate new PyWeak with wr_object pointing to referent
        let weak_payload = PyWeak {
            pointers: Pointers::new(),
            wr_object: Radium::new(obj as *const PyObject as *mut PyObject),
            callback: UnsafeCell::new(callback),
            hash: Radium::new(crate::common::hash::SENTINEL),
        };
        let weak = PyRef::new_ref(weak_payload, cls, dict);

        // Insert into linked list under stripe lock
        let node_ptr = NonNull::from(&*weak);
        unsafe {
            let mut ptrs = WeakLink::pointers(node_ptr);
            if is_generic {
                // Generic ref goes to head (insert_head for basic ref)
                let old_head = self.head.load(Ordering::Relaxed);
                ptrs.as_mut().set_next(NonNull::new(old_head));
                ptrs.as_mut().set_prev(None);
                if let Some(old_head) = NonNull::new(old_head) {
                    WeakLink::pointers(old_head)
                        .as_mut()
                        .set_prev(Some(node_ptr));
                }
                self.head.store(node_ptr.as_ptr(), Ordering::Relaxed);
                self.generic.store(node_ptr.as_ptr(), Ordering::Relaxed);
            } else {
                // Non-generic refs go after generic ref (insert_after)
                let generic_ptr = self.generic.load(Ordering::Relaxed);
                if let Some(after) = NonNull::new(generic_ptr) {
                    let after_next = WeakLink::pointers(after).as_ref().get_next();
                    ptrs.as_mut().set_prev(Some(after));
                    ptrs.as_mut().set_next(after_next);
                    WeakLink::pointers(after).as_mut().set_next(Some(node_ptr));
                    if let Some(next) = after_next {
                        WeakLink::pointers(next).as_mut().set_prev(Some(node_ptr));
                    }
                } else {
                    // No generic ref; insert at head
                    let old_head = self.head.load(Ordering::Relaxed);
                    ptrs.as_mut().set_next(NonNull::new(old_head));
                    ptrs.as_mut().set_prev(None);
                    if let Some(old_head) = NonNull::new(old_head) {
                        WeakLink::pointers(old_head)
                            .as_mut()
                            .set_prev(Some(node_ptr));
                    }
                    self.head.store(node_ptr.as_ptr(), Ordering::Relaxed);
                }
            }
        }

        weak
    }

    /// Clear all weakrefs and call their callbacks.
    /// Called when the owner object is being dropped.
    // PyObject_ClearWeakRefs
    fn clear(&self, obj: &PyObject) {
        let obj_addr = obj as *const PyObject as usize;
        let _lock = weakref_lock::lock(obj_addr);

        // Clear generic cache
        self.generic.store(ptr::null_mut(), Ordering::Relaxed);

        // Walk the list, collecting weakrefs with callbacks
        let mut callbacks: Vec<(PyRef<PyWeak>, PyObjectRef)> = Vec::new();
        let mut current = NonNull::new(self.head.load(Ordering::Relaxed));
        while let Some(node) = current {
            let next = unsafe { WeakLink::pointers(node).as_ref().get_next() };

            let wr = unsafe { node.as_ref() };

            // Mark weakref as dead
            wr.0.payload
                .wr_object
                .store(ptr::null_mut(), Ordering::Relaxed);

            // Unlink from list
            unsafe {
                let mut ptrs = WeakLink::pointers(node);
                ptrs.as_mut().set_prev(None);
                ptrs.as_mut().set_next(None);
            }

            // Collect callback if present and weakref is still alive
            if wr.0.ref_count.get() > 0 {
                let cb = unsafe { wr.0.payload.callback.get().replace(None) };
                if let Some(cb) = cb {
                    callbacks.push((wr.to_owned(), cb));
                }
            }

            current = next;
        }
        self.head.store(ptr::null_mut(), Ordering::Relaxed);

        // Invoke callbacks outside the lock
        drop(_lock);
        for (wr, cb) in callbacks {
            crate::vm::thread::with_vm(&cb, |vm| {
                let _ = cb.call((wr.clone(),), vm);
            });
        }
    }

    /// Clear all weakrefs but DON'T call callbacks. Instead, return them for later invocation.
    /// Used by GC to ensure ALL weakrefs are cleared BEFORE any callbacks are invoked.
    /// handle_weakrefs() clears all weakrefs first, then invokes callbacks.
    fn clear_for_gc_collect_callbacks(&self, obj: &PyObject) -> Vec<(PyRef<PyWeak>, PyObjectRef)> {
        let obj_addr = obj as *const PyObject as usize;
        let _lock = weakref_lock::lock(obj_addr);

        // Clear generic cache
        self.generic.store(ptr::null_mut(), Ordering::Relaxed);

        let mut callbacks = Vec::new();
        let mut current = NonNull::new(self.head.load(Ordering::Relaxed));
        while let Some(node) = current {
            let next = unsafe { WeakLink::pointers(node).as_ref().get_next() };

            let wr = unsafe { node.as_ref() };

            // Mark weakref as dead
            wr.0.payload
                .wr_object
                .store(ptr::null_mut(), Ordering::Relaxed);

            // Unlink from list
            unsafe {
                let mut ptrs = WeakLink::pointers(node);
                ptrs.as_mut().set_prev(None);
                ptrs.as_mut().set_next(None);
            }

            // Collect callback without invoking
            if wr.0.ref_count.get() > 0 {
                let cb = unsafe { wr.0.payload.callback.get().replace(None) };
                if let Some(cb) = cb {
                    callbacks.push((wr.to_owned(), cb));
                }
            }

            current = next;
        }
        self.head.store(ptr::null_mut(), Ordering::Relaxed);

        callbacks
    }

    fn count(&self, obj: &PyObject) -> usize {
        let _lock = weakref_lock::lock(obj as *const PyObject as usize);
        let mut count = 0usize;
        let mut current = NonNull::new(self.head.load(Ordering::Relaxed));
        while let Some(node) = current {
            if unsafe { node.as_ref() }.0.ref_count.get() > 0 {
                count += 1;
            }
            current = unsafe { WeakLink::pointers(node).as_ref().get_next() };
        }
        count
    }

    fn get_weak_references(&self, obj: &PyObject) -> Vec<PyRef<PyWeak>> {
        let _lock = weakref_lock::lock(obj as *const PyObject as usize);
        let mut v = Vec::new();
        let mut current = NonNull::new(self.head.load(Ordering::Relaxed));
        while let Some(node) = current {
            let wr = unsafe { node.as_ref() };
            if wr.0.ref_count.get() > 0 {
                v.push(wr.to_owned());
            }
            current = unsafe { WeakLink::pointers(node).as_ref().get_next() };
        }
        v
    }
}

impl Default for WeakRefList {
    fn default() -> Self {
        Self::new()
    }
}

struct WeakLink;
unsafe impl Link for WeakLink {
    type Handle = PyRef<PyWeak>;

    type Target = Py<PyWeak>;

    #[inline(always)]
    fn as_raw(handle: &PyRef<PyWeak>) -> NonNull<Self::Target> {
        NonNull::from(&**handle)
    }

    #[inline(always)]
    unsafe fn from_raw(ptr: NonNull<Self::Target>) -> Self::Handle {
        unsafe { PyRef::from_raw(ptr.as_ptr()) }
    }

    #[inline(always)]
    unsafe fn pointers(target: NonNull<Self::Target>) -> NonNull<Pointers<Self::Target>> {
        // SAFETY: requirements forwarded from caller
        unsafe { NonNull::new_unchecked(&raw mut (*target.as_ptr()).0.payload.pointers) }
    }
}

/// PyWeakReference: each weakref holds a direct pointer to its referent.
#[pyclass(name = "weakref", module = false)]
#[derive(Debug)]
pub struct PyWeak {
    pointers: Pointers<Py<PyWeak>>,
    /// Direct pointer to the referent object, null when dead.
    /// Equivalent to wr_object in PyWeakReference.
    wr_object: PyAtomic<*mut PyObject>,
    /// Protected by stripe lock (keyed on wr_object address).
    callback: UnsafeCell<Option<PyObjectRef>>,
    pub(crate) hash: PyAtomic<crate::common::hash::PyHash>,
}

cfg_if::cfg_if! {
    if #[cfg(feature = "threading")] {
        #[allow(clippy::non_send_fields_in_send_ty)] // false positive?
        unsafe impl Send for PyWeak {}
        unsafe impl Sync for PyWeak {}
    }
}

impl PyWeak {
    /// _PyWeakref_GET_REF: attempt to upgrade the weakref to a strong reference.
    pub(crate) fn upgrade(&self) -> Option<PyObjectRef> {
        let obj_ptr = self.wr_object.load(Ordering::Acquire);
        if obj_ptr.is_null() {
            return None;
        }

        let _lock = weakref_lock::lock(obj_ptr as usize);

        // Double-check under lock (clear may have run between our check and lock)
        let obj_ptr = self.wr_object.load(Ordering::Relaxed);
        if obj_ptr.is_null() {
            return None;
        }

        unsafe {
            if !(*obj_ptr).0.ref_count.safe_inc() {
                return None;
            }
            Some(PyObjectRef::from_raw(NonNull::new_unchecked(obj_ptr)))
        }
    }

    pub(crate) fn is_dead(&self) -> bool {
        self.wr_object.load(Ordering::Acquire).is_null()
    }

    /// weakref_dealloc: remove from list if still linked.
    fn drop_inner(&self) {
        let obj_ptr = self.wr_object.load(Ordering::Acquire);
        if obj_ptr.is_null() {
            return; // Already cleared by WeakRefList::clear()
        }

        let _lock = weakref_lock::lock(obj_ptr as usize);

        // Double-check under lock
        let obj_ptr = self.wr_object.load(Ordering::Relaxed);
        if obj_ptr.is_null() {
            return; // Cleared between our check and lock acquisition
        }

        let obj = unsafe { &*obj_ptr };
        let wrl = &obj.0.weak_list;

        // Compute our Py<PyWeak> node pointer from payload address
        let offset = std::mem::offset_of!(PyInner<Self>, payload);
        let py_inner = (self as *const Self)
            .cast::<u8>()
            .wrapping_sub(offset)
            .cast::<PyInner<Self>>();
        let node_ptr = unsafe { NonNull::new_unchecked(py_inner as *mut Py<Self>) };

        // Unlink from list
        unsafe { unlink_weakref(wrl, node_ptr) };

        // Update generic cache if this was it
        if wrl.generic.load(Ordering::Relaxed) == node_ptr.as_ptr() {
            wrl.generic.store(ptr::null_mut(), Ordering::Relaxed);
        }

        // Mark as dead
        self.wr_object.store(ptr::null_mut(), Ordering::Relaxed);
    }
}

impl Drop for PyWeak {
    #[inline(always)]
    fn drop(&mut self) {
        // we do NOT have actual exclusive access!
        let me: &Self = self;
        me.drop_inner();
    }
}

impl Py<PyWeak> {
    #[inline(always)]
    pub fn upgrade(&self) -> Option<PyObjectRef> {
        PyWeak::upgrade(self)
    }
}

#[derive(Debug)]
pub(super) struct InstanceDict {
    pub(super) d: PyRwLock<PyDictRef>,
}

impl From<PyDictRef> for InstanceDict {
    #[inline(always)]
    fn from(d: PyDictRef) -> Self {
        Self::new(d)
    }
}

impl InstanceDict {
    #[inline]
    pub const fn new(d: PyDictRef) -> Self {
        Self {
            d: PyRwLock::new(d),
        }
    }

    #[inline]
    pub fn get(&self) -> PyDictRef {
        self.d.read().clone()
    }

    #[inline]
    pub fn set(&self, d: PyDictRef) {
        self.replace(d);
    }

    #[inline]
    pub fn replace(&self, d: PyDictRef) -> PyDictRef {
        core::mem::replace(&mut self.d.write(), d)
    }
}

impl<T: PyPayload + core::fmt::Debug> PyInner<T> {
    fn new(payload: T, typ: PyTypeRef, dict: Option<PyDictRef>) -> Box<Self> {
        let member_count = typ.slots.member_count;
        Box::new(Self {
            ref_count: RefCount::new(),
            vtable: PyObjVTable::of::<T>(),
            gc_bits: Radium::new(0),
            typ: PyAtomicRef::from(typ),
            dict: dict.map(InstanceDict::new),
            weak_list: WeakRefList::new(),
            payload,
            slots: core::iter::repeat_with(|| PyRwLock::new(None))
                .take(member_count)
                .collect_vec()
                .into_boxed_slice(),
        })
    }
}

/// The `PyObjectRef` is one of the most used types. It is a reference to a
/// python object. A single python object can have multiple references, and
/// this reference counting is accounted for by this type. Use the `.clone()`
/// method to create a new reference and increment the amount of references
/// to the python object by 1.
#[repr(transparent)]
pub struct PyObjectRef {
    ptr: NonNull<PyObject>,
}

impl Clone for PyObjectRef {
    #[inline(always)]
    fn clone(&self) -> Self {
        (**self).to_owned()
    }
}

cfg_if::cfg_if! {
    if #[cfg(feature = "threading")] {
        unsafe impl Send for PyObjectRef {}
        unsafe impl Sync for PyObjectRef {}
    }
}

#[repr(transparent)]
pub struct PyObject(PyInner<Erased>);

impl Deref for PyObjectRef {
    type Target = PyObject;

    #[inline(always)]
    fn deref(&self) -> &PyObject {
        unsafe { self.ptr.as_ref() }
    }
}

impl ToOwned for PyObject {
    type Owned = PyObjectRef;

    #[inline(always)]
    fn to_owned(&self) -> Self::Owned {
        self.0.ref_count.inc();
        PyObjectRef {
            ptr: NonNull::from(self),
        }
    }
}

impl PyObjectRef {
    #[inline(always)]
    pub const fn into_raw(self) -> NonNull<PyObject> {
        let ptr = self.ptr;
        core::mem::forget(self);
        ptr
    }

    /// # Safety
    /// The raw pointer must have been previously returned from a call to
    /// [`PyObjectRef::into_raw`]. The user is responsible for ensuring that the inner data is not
    /// dropped more than once due to mishandling the reference count by calling this function
    /// too many times.
    #[inline(always)]
    pub const unsafe fn from_raw(ptr: NonNull<PyObject>) -> Self {
        Self { ptr }
    }

    /// Attempt to downcast this reference to a subclass.
    ///
    /// If the downcast fails, the original ref is returned in as `Err` so
    /// another downcast can be attempted without unnecessary cloning.
    #[inline(always)]
    pub fn downcast<T: PyPayload>(self) -> Result<PyRef<T>, Self> {
        if self.downcastable::<T>() {
            Ok(unsafe { self.downcast_unchecked() })
        } else {
            Err(self)
        }
    }

    pub fn try_downcast<T: PyPayload>(self, vm: &VirtualMachine) -> PyResult<PyRef<T>> {
        T::try_downcast_from(&self, vm)?;
        Ok(unsafe { self.downcast_unchecked() })
    }

    /// Force to downcast this reference to a subclass.
    ///
    /// # Safety
    /// T must be the exact payload type
    #[inline(always)]
    pub unsafe fn downcast_unchecked<T>(self) -> PyRef<T> {
        // PyRef::from_obj_unchecked(self)
        // manual impl to avoid assertion
        let obj = ManuallyDrop::new(self);
        PyRef {
            ptr: obj.ptr.cast(),
        }
    }

    // ideally we'd be able to define these in pyobject.rs, but method visibility rules are weird

    /// Attempt to downcast this reference to the specific class that is associated `T`.
    ///
    /// If the downcast fails, the original ref is returned in as `Err` so
    /// another downcast can be attempted without unnecessary cloning.
    #[inline]
    pub fn downcast_exact<T: PyPayload>(self, vm: &VirtualMachine) -> Result<PyRefExact<T>, Self> {
        if self.class().is(T::class(&vm.ctx)) {
            // TODO: is this always true?
            assert!(
                self.downcastable::<T>(),
                "obj.__class__ is T::class() but payload is not T"
            );
            // SAFETY: just asserted that downcastable::<T>()
            Ok(unsafe { PyRefExact::new_unchecked(PyRef::from_obj_unchecked(self)) })
        } else {
            Err(self)
        }
    }
}

impl PyObject {
    #[inline(always)]
    const fn weak_ref_list(&self) -> Option<&WeakRefList> {
        Some(&self.0.weak_list)
    }

    pub(crate) fn downgrade_with_weakref_typ_opt(
        &self,
        callback: Option<PyObjectRef>,
        // a reference to weakref_type **specifically**
        typ: PyTypeRef,
    ) -> Option<PyRef<PyWeak>> {
        self.weak_ref_list()
            .map(|wrl| wrl.add(self, typ, true, callback, None))
    }

    pub(crate) fn downgrade_with_typ(
        &self,
        callback: Option<PyObjectRef>,
        typ: PyTypeRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<PyWeak>> {
        let dict = if typ
            .slots
            .flags
            .has_feature(crate::types::PyTypeFlags::HAS_DICT)
        {
            Some(vm.ctx.new_dict())
        } else {
            None
        };
        let cls_is_weakref = typ.is(vm.ctx.types.weakref_type);
        let wrl = self.weak_ref_list().ok_or_else(|| {
            vm.new_type_error(format!(
                "cannot create weak reference to '{}' object",
                self.class().name()
            ))
        })?;
        Ok(wrl.add(self, typ, cls_is_weakref, callback, dict))
    }

    pub fn downgrade(
        &self,
        callback: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<PyWeak>> {
        self.downgrade_with_typ(callback, vm.ctx.types.weakref_type.to_owned(), vm)
    }

    pub fn get_weak_references(&self) -> Option<Vec<PyRef<PyWeak>>> {
        self.weak_ref_list()
            .map(|wrl| wrl.get_weak_references(self))
    }

    #[deprecated(note = "use downcastable instead")]
    #[inline(always)]
    pub fn payload_is<T: PyPayload>(&self) -> bool {
        self.0.vtable.typeid == T::PAYLOAD_TYPE_ID
    }

    /// Force to return payload as T.
    ///
    /// # Safety
    /// The actual payload type must be T.
    #[deprecated(note = "use downcast_unchecked_ref instead")]
    #[inline(always)]
    pub const unsafe fn payload_unchecked<T: PyPayload>(&self) -> &T {
        // we cast to a PyInner<T> first because we don't know T's exact offset because of
        // varying alignment, but once we get a PyInner<T> the compiler can get it for us
        let inner = unsafe { &*(&self.0 as *const PyInner<Erased> as *const PyInner<T>) };
        &inner.payload
    }

    #[deprecated(note = "use downcast_ref instead")]
    #[inline(always)]
    pub fn payload<T: PyPayload>(&self) -> Option<&T> {
        #[allow(deprecated)]
        if self.payload_is::<T>() {
            #[allow(deprecated)]
            Some(unsafe { self.payload_unchecked() })
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn class(&self) -> &Py<PyType> {
        self.0.typ.deref()
    }

    pub fn set_class(&self, typ: PyTypeRef, vm: &VirtualMachine) {
        self.0.typ.swap_to_temporary_refs(typ, vm);
    }

    #[deprecated(note = "use downcast_ref_if_exact instead")]
    #[inline(always)]
    pub fn payload_if_exact<T: PyPayload>(&self, vm: &VirtualMachine) -> Option<&T> {
        if self.class().is(T::class(&vm.ctx)) {
            #[allow(deprecated)]
            self.payload()
        } else {
            None
        }
    }

    #[inline(always)]
    const fn instance_dict(&self) -> Option<&InstanceDict> {
        self.0.dict.as_ref()
    }

    #[inline(always)]
    pub fn dict(&self) -> Option<PyDictRef> {
        self.instance_dict().map(|d| d.get())
    }

    /// Set the dict field. Returns `Err(dict)` if this object does not have a dict field
    /// in the first place.
    pub fn set_dict(&self, dict: PyDictRef) -> Result<(), PyDictRef> {
        match self.instance_dict() {
            Some(d) => {
                d.set(dict);
                Ok(())
            }
            None => Err(dict),
        }
    }

    #[deprecated(note = "use downcast_ref instead")]
    #[inline(always)]
    pub fn payload_if_subclass<T: crate::PyPayload>(&self, vm: &VirtualMachine) -> Option<&T> {
        if self.class().fast_issubclass(T::class(&vm.ctx)) {
            #[allow(deprecated)]
            self.payload()
        } else {
            None
        }
    }

    #[inline]
    pub(crate) fn typeid(&self) -> TypeId {
        self.0.vtable.typeid
    }

    /// Check if this object can be downcast to T.
    #[inline(always)]
    pub fn downcastable<T: PyPayload>(&self) -> bool {
        self.typeid() == T::PAYLOAD_TYPE_ID && unsafe { T::validate_downcastable_from(self) }
    }

    /// Attempt to downcast this reference to a subclass.
    pub fn try_downcast_ref<'a, T: PyPayload>(
        &'a self,
        vm: &VirtualMachine,
    ) -> PyResult<&'a Py<T>> {
        T::try_downcast_from(self, vm)?;
        Ok(unsafe { self.downcast_unchecked_ref::<T>() })
    }

    /// Attempt to downcast this reference to a subclass.
    #[inline(always)]
    pub fn downcast_ref<T: PyPayload>(&self) -> Option<&Py<T>> {
        if self.downcastable::<T>() {
            // SAFETY: just checked that the payload is T, and PyRef is repr(transparent) over
            // PyObjectRef
            Some(unsafe { self.downcast_unchecked_ref::<T>() })
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn downcast_ref_if_exact<T: PyPayload>(&self, vm: &VirtualMachine) -> Option<&Py<T>> {
        self.class()
            .is(T::class(&vm.ctx))
            .then(|| unsafe { self.downcast_unchecked_ref::<T>() })
    }

    /// # Safety
    /// T must be the exact payload type
    #[inline(always)]
    pub unsafe fn downcast_unchecked_ref<T: PyPayload>(&self) -> &Py<T> {
        debug_assert!(self.downcastable::<T>());
        // SAFETY: requirements forwarded from caller
        unsafe { &*(self as *const Self as *const Py<T>) }
    }

    #[inline(always)]
    pub fn strong_count(&self) -> usize {
        self.0.ref_count.get()
    }

    #[inline]
    pub fn weak_count(&self) -> Option<usize> {
        self.weak_ref_list().map(|wrl| wrl.count(self))
    }

    #[inline(always)]
    pub const fn as_raw(&self) -> *const Self {
        self
    }

    /// Check if the object has been finalized (__del__ already called).
    /// _PyGC_FINALIZED in Py_GIL_DISABLED mode.
    #[inline]
    pub(crate) fn gc_finalized(&self) -> bool {
        GcBits::from_bits_retain(self.0.gc_bits.load(Ordering::Relaxed)).contains(GcBits::FINALIZED)
    }

    /// Mark the object as finalized. Should be called before __del__.
    /// _PyGC_SET_FINALIZED in Py_GIL_DISABLED mode.
    #[inline]
    fn set_gc_finalized(&self) {
        self.set_gc_bit(GcBits::FINALIZED);
    }

    /// Set a GC bit atomically.
    #[inline]
    pub(crate) fn set_gc_bit(&self, bit: GcBits) {
        self.0.gc_bits.fetch_or(bit.bits(), Ordering::Relaxed);
    }

    /// _PyObject_GC_TRACK
    #[inline]
    pub(crate) fn set_gc_tracked(&self) {
        self.set_gc_bit(GcBits::TRACKED);
    }

    /// _PyObject_GC_UNTRACK
    #[inline]
    pub(crate) fn clear_gc_tracked(&self) {
        self.0
            .gc_bits
            .fetch_and(!GcBits::TRACKED.bits(), Ordering::Relaxed);
    }

    #[inline(always)] // the outer function is never inlined
    fn drop_slow_inner(&self) -> Result<(), ()> {
        // __del__ is mostly not implemented
        #[inline(never)]
        #[cold]
        fn call_slot_del(
            zelf: &PyObject,
            slot_del: fn(&PyObject, &VirtualMachine) -> PyResult<()>,
        ) -> Result<(), ()> {
            let ret = crate::vm::thread::with_vm(zelf, |vm| {
                // Temporarily resurrect (0→2) so ref_count stays positive
                // during __del__, preventing safe_inc from seeing 0.
                zelf.0.ref_count.inc_by(2);

                if let Err(e) = slot_del(zelf, vm) {
                    let del_method = zelf.get_class_attr(identifier!(vm, __del__)).unwrap();
                    vm.run_unraisable(e, None, del_method);
                }

                // Undo the temporary resurrection. Always remove both
                // temporary refs; the second dec returns true only when
                // ref_count drops to 0 (no resurrection).
                zelf.0.ref_count.dec();
                zelf.0.ref_count.dec()
            });
            match ret {
                // the decref set ref_count back to 0
                Some(true) => Ok(()),
                // we've been resurrected by __del__
                Some(false) => Err(()),
                None => {
                    warn!("couldn't run __del__ method for object");
                    Ok(())
                }
            }
        }

        // __del__ should only be called once (like _PyGC_FINALIZED check in GIL_DISABLED)
        // We call __del__ BEFORE clearing weakrefs to allow the finalizer to access
        // the object's weak references if needed.
        let del = self.class().slots.del.load();
        if let Some(slot_del) = del
            && !self.gc_finalized()
        {
            self.set_gc_finalized();
            call_slot_del(self, slot_del)?;
        }

        // Clear weak refs AFTER __del__.
        // Note: This differs from GC behavior which clears weakrefs before finalizers,
        // but for direct deallocation (drop_slow_inner), we need to allow the finalizer
        // to run without triggering use-after-free from WeakRefList operations.
        if let Some(wrl) = self.weak_ref_list() {
            wrl.clear(self);
        }

        Ok(())
    }

    /// _Py_Dealloc: dispatch to type's dealloc
    #[inline(never)]
    unsafe fn drop_slow(ptr: NonNull<Self>) {
        let dealloc = unsafe { ptr.as_ref().0.vtable.dealloc };
        unsafe { dealloc(ptr.as_ptr()) }
    }

    /// # Safety
    /// This call will make the object live forever.
    pub(crate) unsafe fn mark_intern(&self) {
        self.0.ref_count.leak();
    }

    pub(crate) fn is_interned(&self) -> bool {
        self.0.ref_count.is_leaked()
    }

    pub(crate) fn get_slot(&self, offset: usize) -> Option<PyObjectRef> {
        self.0.slots[offset].read().clone()
    }

    pub(crate) fn set_slot(&self, offset: usize, value: Option<PyObjectRef>) {
        *self.0.slots[offset].write() = value;
    }

    /// _PyObject_GC_IS_TRACKED
    pub fn is_gc_tracked(&self) -> bool {
        GcBits::from_bits_retain(self.0.gc_bits.load(Ordering::Relaxed)).contains(GcBits::TRACKED)
    }

    /// Get the referents (objects directly referenced) of this object.
    /// Uses the full traverse including dict and slots.
    pub fn gc_get_referents(&self) -> Vec<PyObjectRef> {
        let mut result = Vec::new();
        self.0.traverse(&mut |child: &PyObject| {
            result.push(child.to_owned());
        });
        result
    }

    /// Call __del__ if present, without triggering object deallocation.
    /// Used by GC to call finalizers before breaking cycles.
    /// This allows proper resurrection detection.
    /// PyObject_CallFinalizerFromDealloc
    pub fn try_call_finalizer(&self) {
        let del = self.class().slots.del.load();
        if let Some(slot_del) = del
            && !self.gc_finalized()
        {
            // Mark as finalized BEFORE calling __del__ to prevent double-call
            // This ensures drop_slow_inner() won't call __del__ again
            self.set_gc_finalized();
            let result = crate::vm::thread::with_vm(self, |vm| {
                if let Err(e) = slot_del(self, vm)
                    && let Some(del_method) = self.get_class_attr(identifier!(vm, __del__))
                {
                    vm.run_unraisable(e, None, del_method);
                }
            });
            let _ = result;
        }
    }

    /// Clear weakrefs but collect callbacks instead of calling them.
    /// This is used by GC to ensure ALL weakrefs are cleared BEFORE any callbacks run.
    /// Returns collected callbacks as (PyRef<PyWeak>, callback) pairs.
    // = handle_weakrefs
    pub fn gc_clear_weakrefs_collect_callbacks(&self) -> Vec<(PyRef<PyWeak>, PyObjectRef)> {
        if let Some(wrl) = self.weak_ref_list() {
            wrl.clear_for_gc_collect_callbacks(self)
        } else {
            vec![]
        }
    }

    /// Get raw pointers to referents without incrementing reference counts.
    /// This is used during GC to avoid reference count manipulation.
    /// tp_traverse visits objects without incref
    ///
    /// # Safety
    /// The returned pointers are only valid as long as the object is alive
    /// and its contents haven't been modified.
    pub unsafe fn gc_get_referent_ptrs(&self) -> Vec<NonNull<PyObject>> {
        let mut result = Vec::new();
        // Traverse the entire object including dict and slots
        self.0.traverse(&mut |child: &PyObject| {
            result.push(NonNull::from(child));
        });
        result
    }

    /// Pop edges from this object for cycle breaking.
    /// Returns extracted child references that were removed from this object (tp_clear).
    /// This is used during garbage collection to break circular references.
    ///
    /// # Safety
    /// - ptr must be a valid pointer to a PyObject
    /// - The caller must have exclusive access (no other references exist)
    /// - This is only safe during GC when the object is unreachable
    pub unsafe fn gc_clear_raw(ptr: *mut PyObject) -> Vec<PyObjectRef> {
        let mut result = Vec::new();
        let obj = unsafe { &*ptr };

        // 1. Clear payload-specific references (vtable.clear / tp_clear)
        if let Some(clear_fn) = obj.0.vtable.clear {
            unsafe { clear_fn(ptr, &mut result) };
        }

        // 2. Clear member slots (subtype_clear)
        for slot in obj.0.slots.iter() {
            if let Some(val) = slot.write().take() {
                result.push(val);
            }
        }

        result
    }

    /// Clear this object for cycle breaking (tp_clear).
    /// This version takes &self but should only be called during GC
    /// when exclusive access is guaranteed.
    ///
    /// # Safety
    /// - The caller must guarantee exclusive access (no other references exist)
    /// - This is only safe during GC when the object is unreachable
    pub unsafe fn gc_clear(&self) -> Vec<PyObjectRef> {
        // SAFETY: During GC collection, this object is unreachable (gc_refs == 0),
        // meaning no other code has a reference to it. The only references are
        // internal cycle references which we're about to break.
        unsafe { Self::gc_clear_raw(self as *const _ as *mut PyObject) }
    }

    /// Check if this object has clear capability (tp_clear)
    // Py_TPFLAGS_HAVE_GC types have tp_clear
    pub fn gc_has_clear(&self) -> bool {
        self.0.vtable.clear.is_some() || self.0.dict.is_some() || !self.0.slots.is_empty()
    }
}

impl Borrow<PyObject> for PyObjectRef {
    #[inline(always)]
    fn borrow(&self) -> &PyObject {
        self
    }
}

impl AsRef<PyObject> for PyObjectRef {
    #[inline(always)]
    fn as_ref(&self) -> &PyObject {
        self
    }
}

impl<'a, T: PyPayload> From<&'a Py<T>> for &'a PyObject {
    #[inline(always)]
    fn from(py_ref: &'a Py<T>) -> Self {
        py_ref.as_object()
    }
}

impl Drop for PyObjectRef {
    #[inline]
    fn drop(&mut self) {
        if self.0.ref_count.dec() {
            unsafe { PyObject::drop_slow(self.ptr) }
        }
    }
}

impl fmt::Debug for PyObject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // SAFETY: the vtable contains functions that accept payload types that always match up
        // with the payload of the object
        unsafe { (self.0.vtable.debug)(self, f) }
    }
}

impl fmt::Debug for PyObjectRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_object().fmt(f)
    }
}

#[repr(transparent)]
pub struct Py<T>(PyInner<T>);

impl<T: PyPayload> Py<T> {
    pub fn downgrade(
        &self,
        callback: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyWeakRef<T>> {
        Ok(PyWeakRef {
            weak: self.as_object().downgrade(callback, vm)?,
            _marker: PhantomData,
        })
    }

    #[inline]
    pub fn payload(&self) -> &T {
        &self.0.payload
    }
}

impl<T> ToOwned for Py<T> {
    type Owned = PyRef<T>;

    #[inline(always)]
    fn to_owned(&self) -> Self::Owned {
        self.0.ref_count.inc();
        PyRef {
            ptr: NonNull::from(self),
        }
    }
}

impl<T> Deref for Py<T> {
    type Target = T;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        &self.0.payload
    }
}

impl<T: PyPayload> Borrow<PyObject> for Py<T> {
    #[inline(always)]
    fn borrow(&self) -> &PyObject {
        unsafe { &*(&self.0 as *const PyInner<T> as *const PyObject) }
    }
}

impl<T> core::hash::Hash for Py<T>
where
    T: core::hash::Hash + PyPayload,
{
    #[inline]
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.deref().hash(state)
    }
}

impl<T> PartialEq for Py<T>
where
    T: PartialEq + PyPayload,
{
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.deref().eq(other.deref())
    }
}

impl<T> Eq for Py<T> where T: Eq + PyPayload {}

impl<T> AsRef<PyObject> for Py<T>
where
    T: PyPayload,
{
    #[inline(always)]
    fn as_ref(&self) -> &PyObject {
        self.borrow()
    }
}

impl<T: PyPayload + core::fmt::Debug> fmt::Debug for Py<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

/// A reference to a Python object.
///
/// Note that a `PyRef<T>` can only deref to a shared / immutable reference.
/// It is the payload type's responsibility to handle (possibly concurrent)
/// mutability with locks or concurrent data structures if required.
///
/// A `PyRef<T>` can be directly returned from a built-in function to handle
/// situations (such as when implementing in-place methods such as `__iadd__`)
/// where a reference to the same object must be returned.
#[repr(transparent)]
pub struct PyRef<T> {
    ptr: NonNull<Py<T>>,
}

cfg_if::cfg_if! {
    if #[cfg(feature = "threading")] {
        unsafe impl<T> Send for PyRef<T> {}
        unsafe impl<T> Sync for PyRef<T> {}
    }
}

impl<T: fmt::Debug> fmt::Debug for PyRef<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

impl<T> Drop for PyRef<T> {
    #[inline]
    fn drop(&mut self) {
        if self.0.ref_count.dec() {
            unsafe { PyObject::drop_slow(self.ptr.cast::<PyObject>()) }
        }
    }
}

impl<T> Clone for PyRef<T> {
    #[inline(always)]
    fn clone(&self) -> Self {
        (**self).to_owned()
    }
}

impl<T: PyPayload> PyRef<T> {
    // #[inline(always)]
    // pub(crate) const fn into_non_null(self) -> NonNull<Py<T>> {
    //     let ptr = self.ptr;
    //     std::mem::forget(self);
    //     ptr
    // }

    #[inline(always)]
    pub(crate) const unsafe fn from_non_null(ptr: NonNull<Py<T>>) -> Self {
        Self { ptr }
    }

    /// # Safety
    /// The raw pointer must point to a valid `Py<T>` object
    #[inline(always)]
    pub(crate) const unsafe fn from_raw(raw: *const Py<T>) -> Self {
        unsafe { Self::from_non_null(NonNull::new_unchecked(raw as *mut _)) }
    }

    /// Safety: payload type of `obj` must be `T`
    #[inline(always)]
    unsafe fn from_obj_unchecked(obj: PyObjectRef) -> Self {
        debug_assert!(obj.downcast_ref::<T>().is_some());
        let obj = ManuallyDrop::new(obj);
        Self {
            ptr: obj.ptr.cast(),
        }
    }

    pub const fn leak(pyref: Self) -> &'static Py<T> {
        let ptr = pyref.ptr;
        core::mem::forget(pyref);
        unsafe { ptr.as_ref() }
    }
}

impl<T: PyPayload + crate::object::MaybeTraverse + core::fmt::Debug> PyRef<T> {
    #[inline(always)]
    pub fn new_ref(payload: T, typ: crate::builtins::PyTypeRef, dict: Option<PyDictRef>) -> Self {
        let has_dict = dict.is_some();
        let is_heaptype = typ.heaptype_ext.is_some();
        let inner = Box::into_raw(PyInner::new(payload, typ, dict));
        let ptr = unsafe { NonNull::new_unchecked(inner.cast::<Py<T>>()) };

        // Track object if:
        // - HAS_TRAVERSE is true (Rust payload implements Traverse), OR
        // - has instance dict (user-defined class instances), OR
        // - heap type (all heap type instances are GC-tracked, like Py_TPFLAGS_HAVE_GC)
        if <T as crate::object::MaybeTraverse>::HAS_TRAVERSE || has_dict || is_heaptype {
            let gc = crate::gc_state::gc_state();
            unsafe {
                gc.track_object(ptr.cast());
            }
            // Check if automatic GC should run
            gc.maybe_collect();
        }

        Self { ptr }
    }
}

impl<T: crate::class::PySubclass + core::fmt::Debug> PyRef<T>
where
    T::Base: core::fmt::Debug,
{
    /// Converts this reference to the base type (ownership transfer).
    /// # Safety
    /// T and T::Base must have compatible layouts in size_of::<T::Base>() bytes.
    #[inline]
    pub fn into_base(self) -> PyRef<T::Base> {
        let obj: PyObjectRef = self.into();
        match obj.downcast() {
            Ok(base_ref) => base_ref,
            Err(_) => unsafe { core::hint::unreachable_unchecked() },
        }
    }
    #[inline]
    pub fn upcast<U: PyPayload + StaticType>(self) -> PyRef<U>
    where
        T: StaticType,
    {
        debug_assert!(T::static_type().is_subtype(U::static_type()));
        let obj: PyObjectRef = self.into();
        match obj.downcast::<U>() {
            Ok(upcast_ref) => upcast_ref,
            Err(_) => unsafe { core::hint::unreachable_unchecked() },
        }
    }
}

impl<T: crate::class::PySubclass> Py<T> {
    /// Converts `&Py<T>` to `&Py<T::Base>`.
    #[inline]
    pub fn to_base(&self) -> &Py<T::Base> {
        debug_assert!(self.as_object().downcast_ref::<T::Base>().is_some());
        // SAFETY: T is #[repr(transparent)] over T::Base,
        // so Py<T> and Py<T::Base> have the same layout.
        unsafe { &*(self as *const Py<T> as *const Py<T::Base>) }
    }

    /// Converts `&Py<T>` to `&Py<U>` where U is an ancestor type.
    #[inline]
    pub fn upcast_ref<U: PyPayload + StaticType>(&self) -> &Py<U>
    where
        T: StaticType,
    {
        debug_assert!(T::static_type().is_subtype(U::static_type()));
        // SAFETY: T is a subtype of U, so Py<T> can be viewed as Py<U>.
        unsafe { &*(self as *const Py<T> as *const Py<U>) }
    }
}

impl<T> Borrow<PyObject> for PyRef<T>
where
    T: PyPayload,
{
    #[inline(always)]
    fn borrow(&self) -> &PyObject {
        (**self).as_object()
    }
}

impl<T> AsRef<PyObject> for PyRef<T>
where
    T: PyPayload,
{
    #[inline(always)]
    fn as_ref(&self) -> &PyObject {
        self.borrow()
    }
}

impl<T> From<PyRef<T>> for PyObjectRef {
    #[inline]
    fn from(value: PyRef<T>) -> Self {
        let me = ManuallyDrop::new(value);
        Self { ptr: me.ptr.cast() }
    }
}

impl<T> Borrow<Py<T>> for PyRef<T> {
    #[inline(always)]
    fn borrow(&self) -> &Py<T> {
        self
    }
}

impl<T> AsRef<Py<T>> for PyRef<T> {
    #[inline(always)]
    fn as_ref(&self) -> &Py<T> {
        self
    }
}

impl<T> Deref for PyRef<T> {
    type Target = Py<T>;

    #[inline(always)]
    fn deref(&self) -> &Py<T> {
        unsafe { self.ptr.as_ref() }
    }
}

impl<T> core::hash::Hash for PyRef<T>
where
    T: core::hash::Hash + PyPayload,
{
    #[inline]
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.deref().hash(state)
    }
}

impl<T> PartialEq for PyRef<T>
where
    T: PartialEq + PyPayload,
{
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.deref().eq(other.deref())
    }
}

impl<T> Eq for PyRef<T> where T: Eq + PyPayload {}

#[repr(transparent)]
pub struct PyWeakRef<T: PyPayload> {
    weak: PyRef<PyWeak>,
    _marker: PhantomData<T>,
}

impl<T: PyPayload> PyWeakRef<T> {
    pub fn upgrade(&self) -> Option<PyRef<T>> {
        self.weak
            .upgrade()
            // SAFETY: PyWeakRef<T> was always created from a PyRef<T>, so the object is T
            .map(|obj| unsafe { PyRef::from_obj_unchecked(obj) })
    }
}

/// Partially initialize a struct, ensuring that all fields are
/// either given values or explicitly left uninitialized
macro_rules! partially_init {
    (
        $ty:path {$($init_field:ident: $init_value:expr),*$(,)?},
        Uninit { $($uninit_field:ident),*$(,)? }$(,)?
    ) => {{
        // check all the fields are there but *don't* actually run it

        #[allow(clippy::diverging_sub_expression)]  // FIXME: better way than using `if false`?
        if false {
            #[allow(invalid_value, dead_code, unreachable_code)]
            let _ = {$ty {
                $($init_field: $init_value,)*
                $($uninit_field: unreachable!(),)*
            }};
        }
        let mut m = ::core::mem::MaybeUninit::<$ty>::uninit();
        #[allow(unused_unsafe)]
        unsafe {
            $(::core::ptr::write(&mut (*m.as_mut_ptr()).$init_field, $init_value);)*
        }
        m
    }};
}

pub(crate) fn init_type_hierarchy() -> (PyTypeRef, PyTypeRef, PyTypeRef) {
    use crate::{builtins::object, class::PyClassImpl};
    use core::mem::MaybeUninit;

    // `type` inherits from `object`
    // and both `type` and `object are instances of `type`.
    // to produce this circular dependency, we need an unsafe block.
    // (and yes, this will never get dropped. TODO?)
    let (type_type, object_type) = {
        // We cast between these 2 types, so make sure (at compile time) that there's no change in
        // layout when we wrap PyInner<PyTypeObj> in MaybeUninit<>
        static_assertions::assert_eq_size!(MaybeUninit<PyInner<PyType>>, PyInner<PyType>);
        static_assertions::assert_eq_align!(MaybeUninit<PyInner<PyType>>, PyInner<PyType>);

        let type_payload = PyType {
            base: None,
            bases: PyRwLock::default(),
            mro: PyRwLock::default(),
            subclasses: PyRwLock::default(),
            attributes: PyRwLock::new(Default::default()),
            slots: PyType::make_slots(),
            heaptype_ext: None,
        };
        let object_payload = PyType {
            base: None,
            bases: PyRwLock::default(),
            mro: PyRwLock::default(),
            subclasses: PyRwLock::default(),
            attributes: PyRwLock::new(Default::default()),
            slots: object::PyBaseObject::make_slots(),
            heaptype_ext: None,
        };
        let type_type_ptr = Box::into_raw(Box::new(partially_init!(
            PyInner::<PyType> {
                ref_count: RefCount::new(),
                vtable: PyObjVTable::of::<PyType>(),
                gc_bits: Radium::new(0),
                dict: None,
                weak_list: WeakRefList::new(),
                payload: type_payload,
                slots: Box::new([]),
            },
            Uninit { typ }
        )));
        let object_type_ptr = Box::into_raw(Box::new(partially_init!(
            PyInner::<PyType> {
                ref_count: RefCount::new(),
                vtable: PyObjVTable::of::<PyType>(),
                gc_bits: Radium::new(0),
                dict: None,
                weak_list: WeakRefList::new(),
                payload: object_payload,
                slots: Box::new([]),
            },
            Uninit { typ },
        )));

        let object_type_ptr = object_type_ptr as *mut PyInner<PyType>;
        let type_type_ptr = type_type_ptr as *mut PyInner<PyType>;

        unsafe {
            (*type_type_ptr).ref_count.inc();
            let type_type = PyTypeRef::from_raw(type_type_ptr.cast());
            ptr::write(&mut (*object_type_ptr).typ, PyAtomicRef::from(type_type));
            (*type_type_ptr).ref_count.inc();
            let type_type = PyTypeRef::from_raw(type_type_ptr.cast());
            ptr::write(&mut (*type_type_ptr).typ, PyAtomicRef::from(type_type));

            let object_type = PyTypeRef::from_raw(object_type_ptr.cast());
            // object's mro is [object]
            (*object_type_ptr).payload.mro = PyRwLock::new(vec![object_type.clone()]);

            (*type_type_ptr).payload.bases = PyRwLock::new(vec![object_type.clone()]);
            (*type_type_ptr).payload.base = Some(object_type.clone());

            let type_type = PyTypeRef::from_raw(type_type_ptr.cast());
            // type's mro is [type, object]
            (*type_type_ptr).payload.mro =
                PyRwLock::new(vec![type_type.clone(), object_type.clone()]);

            (type_type, object_type)
        }
    };

    let weakref_type = PyType {
        base: Some(object_type.clone()),
        bases: PyRwLock::new(vec![object_type.clone()]),
        mro: PyRwLock::new(vec![object_type.clone()]),
        subclasses: PyRwLock::default(),
        attributes: PyRwLock::default(),
        slots: PyWeak::make_slots(),
        heaptype_ext: None,
    };
    let weakref_type = PyRef::new_ref(weakref_type, type_type.clone(), None);
    // Static type: untrack from GC (was tracked by new_ref because PyType has HAS_TRAVERSE)
    unsafe {
        crate::gc_state::gc_state()
            .untrack_object(core::ptr::NonNull::from(weakref_type.as_object()));
    }
    weakref_type.as_object().clear_gc_tracked();
    // weakref's mro is [weakref, object]
    weakref_type.mro.write().insert(0, weakref_type.clone());

    object_type.subclasses.write().push(
        type_type
            .as_object()
            .downgrade_with_weakref_typ_opt(None, weakref_type.clone())
            .unwrap(),
    );

    object_type.subclasses.write().push(
        weakref_type
            .as_object()
            .downgrade_with_weakref_typ_opt(None, weakref_type.clone())
            .unwrap(),
    );

    (type_type, object_type, weakref_type)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn miri_test_type_initialization() {
        let _ = init_type_hierarchy();
    }

    #[test]
    fn miri_test_drop() {
        //cspell:ignore dfghjkl
        let ctx = crate::Context::genesis();
        let obj = ctx.new_bytes(b"dfghjkl".to_vec());
        drop(obj);
    }
}
