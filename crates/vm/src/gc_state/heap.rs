//! Object membership and lifetime, independent of an interpreter's lifetime.

use super::*;
use crate::common::rc::PyRc;
use core::sync::atomic::{AtomicIsize, AtomicPtr};

const NO_SLOT: usize = usize::MAX;

/// A strong heap handle and a slot, replacing the two intrusive list pointers.
/// The heap owns only raw object pointers: this strong handle creates no cycle.
/// Keeping it until object destruction makes lookup safe even after untracking
/// or interpreter shutdown, without a registry lookup on the deallocation path.
pub(crate) struct GcHandle {
    heap: AtomicPtr<GcHeap>,
    slot: AtomicUsize,
}

impl GcHandle {
    pub(crate) const fn new() -> Self {
        Self {
            heap: AtomicPtr::new(core::ptr::null_mut()),
            slot: AtomicUsize::new(NO_SLOT),
        }
    }

    pub(crate) fn heap(&self) -> Option<&GcHeap> {
        // The pointer is assigned once before publishing the object and owns
        // a strong reference until destruction. A borrow of the object pins it.
        unsafe { self.heap.load(Ordering::Acquire).as_ref() }
    }

    /// Bind a fresh allocation before it is published, including objects that
    /// never enter the cycle collector but still need owner-bound destruction.
    pub(crate) fn bind_new(&self, heap: PyRc<GcHeap>) {
        debug_assert!(self.heap.load(Ordering::Relaxed).is_null());
        self.heap
            .store(PyRc::into_raw(heap).cast_mut(), Ordering::Release);
    }

    fn bind(&self, heap: &PyRc<GcHeap>) -> &GcHeap {
        if self.heap().is_none() {
            let raw = PyRc::into_raw(heap.clone()).cast_mut();
            if self
                .heap
                .compare_exchange(
                    core::ptr::null_mut(),
                    raw,
                    Ordering::Release,
                    Ordering::Relaxed,
                )
                .is_err()
            {
                unsafe { drop(PyRc::from_raw(raw)) };
            }
        }
        self.heap().unwrap()
    }

    /// Release before publishing a husk in a freelist; Drop is the backstop
    /// for objects that bypass freelists, including bootstrap allocations.
    pub(crate) fn release(&self) {
        debug_assert_eq!(self.slot.load(Ordering::Relaxed), NO_SLOT);
        let ptr = self.heap.swap(core::ptr::null_mut(), Ordering::Relaxed);
        if !ptr.is_null() {
            unsafe { drop(PyRc::from_raw(ptr)) };
        }
    }
}

impl Drop for GcHandle {
    fn drop(&mut self) {
        self.release();
    }
}

#[derive(Clone, Copy)]
pub(super) struct ObjectPtr(pub NonNull<PyObject>);

// Access is protected by the heap lock and collection's stop-the-world barrier.
#[cfg(feature = "threading")]
unsafe impl Send for ObjectPtr {}
#[cfg(feature = "threading")]
unsafe impl Sync for ObjectPtr {}

pub(super) struct HeapObjects {
    generations: [Vec<ObjectPtr>; 4],
}

impl HeapObjects {
    fn insert(&mut self, obj: &PyObject, generation: usize) {
        let objects = &mut self.generations[generation];
        obj.gc_handle().slot.store(objects.len(), Ordering::Relaxed);
        objects.push(ObjectPtr(NonNull::from(obj)));
        obj.set_gc_generation(generation as u8);
    }

    fn remove(&mut self, obj: &PyObject) -> bool {
        let generation = obj.gc_generation() as usize;
        if generation >= self.generations.len() {
            return false;
        }
        let objects = &mut self.generations[generation];
        let slot = obj.gc_handle().slot.load(Ordering::Relaxed);
        assert_eq!(objects[slot].0, NonNull::from(obj));
        objects.swap_remove(slot);
        if let Some(moved) = objects.get(slot) {
            unsafe { moved.0.as_ref() }
                .gc_handle()
                .slot
                .store(slot, Ordering::Relaxed);
        }
        obj.gc_handle().slot.store(NO_SLOT, Ordering::Relaxed);
        obj.set_gc_generation(GC_UNTRACKED);
        true
    }

    fn move_all(&mut self, from: usize, to: usize) {
        let mut objects = core::mem::take(&mut self.generations[from]);
        self.generations[to].reserve(objects.len());
        for ptr in objects.drain(..) {
            self.insert(unsafe { ptr.0.as_ref() }, to);
        }
        // Empty generations do not retain a previous full heap's allocation.
    }
}

/// A heap outlives its VM while any object allocated into it still exists.
pub struct GcHeap {
    pub(crate) owner: Option<i64>,
    #[cfg(feature = "threading")]
    pub(crate) qsbr: alloc::sync::Arc<crate::object::qsbr::Qsbr>,
    objects: PyMutex<HeapObjects>,
    // Shared immortal referents never link weakrefs from different owners.
    // Keeping their native list heads here leaves ordinary object prefixes
    // unchanged and releases the directory with its owner heap.
    weakrefs: PyMutex<std::collections::HashMap<usize, PyRc<crate::object::WeakRefList>>>,
    pub(super) allocations: AtomicIsize,
    pub(super) retired: AtomicBool,
}

impl GcHeap {
    pub(super) fn new(owner: Option<i64>) -> Self {
        Self {
            owner,
            #[cfg(feature = "threading")]
            qsbr: if owner.is_some() {
                alloc::sync::Arc::new(crate::object::qsbr::Qsbr::new())
            } else {
                crate::object::qsbr::shared().clone()
            },
            objects: PyMutex::new(HeapObjects {
                generations: core::array::from_fn(|_| Vec::new()),
            }),
            weakrefs: PyMutex::new(std::collections::HashMap::new()),
            allocations: AtomicIsize::new(0),
            retired: AtomicBool::new(false),
        }
    }

    pub(crate) fn weakrefs(&self, referent: &PyObject) -> PyRc<crate::object::WeakRefList> {
        debug_assert!(referent.is_immortal());
        self.weakrefs
            .lock()
            .entry(core::ptr::from_ref(referent).addr())
            .or_default()
            .clone()
    }

    /// The caller owns a valid, untracked object and excludes concurrent tracking.
    pub(super) unsafe fn track(heap: &PyRc<Self>, ptr: NonNull<PyObject>, fresh: bool) {
        let obj = unsafe { ptr.as_ref() };
        let heap = obj.gc_handle().bind(heap);
        let mut objects = heap.objects.lock();
        assert!(!obj.is_gc_tracked());
        if fresh {
            obj.init_gc_tracked_bit();
        } else {
            obj.set_gc_tracked();
        }
        objects.insert(obj, 0);
        heap.allocations.fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn untrack(&self, obj: &PyObject) {
        let mut objects = self.objects.lock();
        if objects.remove(obj) {
            obj.clear_gc_tracked();
            self.allocations.fetch_sub(1, Ordering::Relaxed);
        }
    }

    /// Publish a freshly allocated generator and frame under one heap lock.
    pub(super) unsafe fn track_pair(
        heap: &PyRc<Self>,
        first: NonNull<PyObject>,
        second: NonNull<PyObject>,
    ) {
        let first = unsafe { first.as_ref() };
        let second = unsafe { second.as_ref() };
        first.gc_handle().bind(heap);
        second.gc_handle().bind(heap);
        let mut objects = heap.objects.lock();
        for obj in [first, second] {
            debug_assert!(core::ptr::eq(obj.gc_handle().heap().unwrap(), &**heap));
            obj.init_gc_tracked_bit();
            objects.insert(obj, 0);
        }
        heap.allocations.fetch_add(2, Ordering::Relaxed);
    }

    pub(super) fn snapshot(&self, generation: Option<usize>) -> Vec<PyObjectRef> {
        let objects = self.objects.lock();
        let end = generation.map_or(GC_PERMANENT as usize, |g| g + 1);
        objects.generations[..end]
            .iter()
            .flatten()
            .filter_map(|p| unsafe { p.0.as_ref() }.try_to_owned())
            .collect()
    }

    pub(super) fn get_objects(&self, generation: Option<i32>) -> Vec<PyObjectRef> {
        let objects = self.objects.lock();
        let range = match generation {
            None => 0..GC_PERMANENT as usize,
            Some(g) if (0..=2).contains(&g) => g as usize..g as usize + 1,
            _ => return Vec::new(),
        };
        objects.generations[range]
            .iter()
            .flatten()
            .filter_map(|p| unsafe { p.0.as_ref() }.try_to_owned())
            .collect()
    }

    pub(super) fn promote(&self, objects: &[PyObjectRef], generation: usize) {
        let destination = (generation + 1).min(2);
        for batch in objects.chunks(256) {
            let mut storage = self.objects.lock();
            for obj in batch {
                if obj.gc_generation() as usize >= destination {
                    continue;
                }
                if storage.remove(obj) {
                    storage.insert(obj, destination);
                }
            }
        }
    }

    pub(super) fn freeze(&self) {
        let mut objects = self.objects.lock();
        for generation in 0..GC_PERMANENT as usize {
            objects.move_all(generation, GC_PERMANENT as usize);
        }
        self.allocations.store(0, Ordering::Relaxed);
    }

    pub(super) fn unfreeze(&self) {
        self.objects.lock().move_all(GC_PERMANENT as usize, 2);
    }

    pub(super) fn freeze_count(&self) -> usize {
        self.objects.lock().generations[GC_PERMANENT as usize].len()
    }

    pub(super) fn trim(&self) {
        let mut objects = self.objects.lock();
        for generation in &mut objects.generations {
            if generation.capacity() > generation.len().saturating_mul(4).max(256) {
                generation.shrink_to_fit();
            }
        }
    }

    #[cfg(all(unix, feature = "threading"))]
    pub(super) unsafe fn reinit_after_fork(&self) {
        unsafe {
            crate::common::lock::reinit_mutex_after_fork(&self.objects);
            crate::common::lock::reinit_mutex_after_fork(&self.weakrefs);
        }
    }
}

impl Drop for GcHeap {
    fn drop(&mut self) {
        debug_assert!(self.objects.get_mut().generations.iter().all(Vec::is_empty));
    }
}
