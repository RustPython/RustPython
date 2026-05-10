//! Garbage Collection State and Algorithm
//!
//! Generational garbage collection using an intrusive doubly-linked list.

use crate::common::linked_list::LinkedList;
use crate::common::lock::{PyMutex, PyRwLock};
use crate::object::{GC_PERMANENT, GC_UNTRACKED, GcLink};
use crate::{AsObject, PyObject, PyObjectRef};
use core::ptr::NonNull;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::collections::HashSet;

#[cfg(not(target_arch = "wasm32"))]
fn elapsed_secs(start: &std::time::Instant) -> f64 {
    start.elapsed().as_secs_f64()
}

#[cfg(target_arch = "wasm32")]
fn elapsed_secs(_start: &()) -> f64 {
    0.0
}

bitflags::bitflags! {
    /// GC debug flags (see Include/internal/pycore_gc.h)
    #[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
    pub struct GcDebugFlags: u32 {
        /// Print collection statistics
        const STATS         = 1 << 0;
        /// Print collectable objects
        const COLLECTABLE   = 1 << 1;
        /// Print uncollectable objects
        const UNCOLLECTABLE = 1 << 2;
        /// Save all garbage in gc.garbage
        const SAVEALL       = 1 << 5;
        /// DEBUG_COLLECTABLE | DEBUG_UNCOLLECTABLE | DEBUG_SAVEALL
        const LEAK = Self::COLLECTABLE.bits() | Self::UNCOLLECTABLE.bits() | Self::SAVEALL.bits();
    }
}

/// Result from a single collection run
#[derive(Debug, Default)]
pub struct CollectResult {
    pub collected: usize,
    pub uncollectable: usize,
    pub candidates: usize,
    pub duration: f64,
}

/// Statistics for a single generation (gc_generation_stats)
#[derive(Debug, Default)]
pub struct GcStats {
    pub collections: usize,
    pub collected: usize,
    pub uncollectable: usize,
    pub candidates: usize,
    pub duration: f64,
}

/// A single GC generation with intrusive linked list
pub struct GcGeneration {
    /// Number of objects in this generation
    count: AtomicUsize,
    /// Threshold for triggering collection
    threshold: AtomicU32,
    /// Collection statistics
    stats: PyMutex<GcStats>,
}

impl GcGeneration {
    #[must_use]
    pub const fn new(threshold: u32) -> Self {
        Self {
            count: AtomicUsize::new(0),
            threshold: AtomicU32::new(threshold),
            stats: PyMutex::new(GcStats {
                collections: 0,
                collected: 0,
                uncollectable: 0,
                candidates: 0,
                duration: 0.0,
            }),
        }
    }

    pub fn count(&self) -> usize {
        self.count.load(Ordering::SeqCst)
    }

    pub fn threshold(&self) -> u32 {
        self.threshold.load(Ordering::SeqCst)
    }

    pub fn set_threshold(&self, value: u32) {
        self.threshold.store(value, Ordering::SeqCst);
    }

    pub fn stats(&self) -> GcStats {
        let guard = self.stats.lock();
        GcStats {
            collections: guard.collections,
            collected: guard.collected,
            uncollectable: guard.uncollectable,
            candidates: guard.candidates,
            duration: guard.duration,
        }
    }

    pub fn update_stats(
        &self,
        collected: usize,
        uncollectable: usize,
        candidates: usize,
        duration: f64,
    ) {
        let mut guard = self.stats.lock();
        guard.collections += 1;
        guard.collected += collected;
        guard.uncollectable += uncollectable;
        guard.candidates += candidates;
        guard.duration += duration;
    }

    /// Reset the stats mutex to unlocked state after fork().
    ///
    /// # Safety
    /// Must only be called after fork() in the child process when no other
    /// threads exist.
    #[cfg(all(unix, feature = "threading"))]
    unsafe fn reinit_stats_after_fork(&self) {
        unsafe { crate::common::lock::reinit_mutex_after_fork(&self.stats) };
    }
}

/// Wrapper for NonNull<PyObject> to impl Hash/Eq for use in temporary collection sets.
/// Only used within collect_inner, never shared across threads.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct GcPtr(NonNull<PyObject>);

/// Global GC state
pub struct GcState {
    /// 3 generations (0 = youngest, 2 = oldest)
    pub generations: [GcGeneration; 3],
    /// Permanent generation (frozen objects)
    pub permanent: GcGeneration,
    /// GC enabled flag
    pub enabled: AtomicBool,
    /// Per-generation intrusive linked lists for object tracking.
    /// Objects start in gen0, survivors are promoted to gen1, then gen2.
    generation_lists: [PyRwLock<LinkedList<GcLink, PyObject>>; 3],
    /// Frozen/permanent objects (excluded from normal GC)
    permanent_list: PyRwLock<LinkedList<GcLink, PyObject>>,
    /// Debug flags
    pub debug: AtomicU32,
    /// gc.garbage list (uncollectable objects with __del__)
    pub garbage: PyMutex<Vec<PyObjectRef>>,
    /// gc.callbacks list
    pub callbacks: PyMutex<Vec<PyObjectRef>>,
    /// Mutex for collection (prevents concurrent collections)
    collecting: PyMutex<()>,
    /// Allocation counter for gen0
    alloc_count: AtomicUsize,
}

// SAFETY: All fields are either inherently Send/Sync (atomics, RwLock, Mutex) or protected by PyMutex.
// LinkedList<GcLink, PyObject> is Send+Sync because GcLink's Target (PyObject) is Send+Sync.
#[cfg(feature = "threading")]
unsafe impl Send for GcState {}
#[cfg(feature = "threading")]
unsafe impl Sync for GcState {}

impl Default for GcState {
    fn default() -> Self {
        Self::new()
    }
}

impl GcState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            generations: [
                GcGeneration::new(2000), // young
                GcGeneration::new(10),   // old[0]
                GcGeneration::new(0),    // old[1]
            ],
            permanent: GcGeneration::new(0),
            enabled: AtomicBool::new(true),
            generation_lists: [
                PyRwLock::new(LinkedList::new()),
                PyRwLock::new(LinkedList::new()),
                PyRwLock::new(LinkedList::new()),
            ],
            permanent_list: PyRwLock::new(LinkedList::new()),
            debug: AtomicU32::new(0),
            garbage: PyMutex::new(Vec::new()),
            callbacks: PyMutex::new(Vec::new()),
            collecting: PyMutex::new(()),
            alloc_count: AtomicUsize::new(0),
        }
    }

    /// Check if GC is enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    /// Enable GC
    pub fn enable(&self) {
        self.enabled.store(true, Ordering::SeqCst);
    }

    /// Disable GC
    pub fn disable(&self) {
        self.enabled.store(false, Ordering::SeqCst);
    }

    /// Get debug flags
    pub fn get_debug(&self) -> GcDebugFlags {
        GcDebugFlags::from_bits_truncate(self.debug.load(Ordering::SeqCst))
    }

    /// Set debug flags
    pub fn set_debug(&self, flags: GcDebugFlags) {
        self.debug.store(flags.bits(), Ordering::SeqCst);
    }

    /// Get thresholds for all generations
    pub fn get_threshold(&self) -> (u32, u32, u32) {
        (
            self.generations[0].threshold(),
            self.generations[1].threshold(),
            self.generations[2].threshold(),
        )
    }

    /// Set thresholds
    pub fn set_threshold(&self, t0: u32, t1: Option<u32>, t2: Option<u32>) {
        self.generations[0].set_threshold(t0);
        if let Some(t1) = t1 {
            self.generations[1].set_threshold(t1);
        }
        if let Some(t2) = t2 {
            self.generations[2].set_threshold(t2);
        }
    }

    /// Get counts for all generations
    pub fn get_count(&self) -> (usize, usize, usize) {
        (
            self.generations[0].count(),
            self.generations[1].count(),
            self.generations[2].count(),
        )
    }

    /// Get statistics for all generations
    pub fn get_stats(&self) -> [GcStats; 3] {
        [
            self.generations[0].stats(),
            self.generations[1].stats(),
            self.generations[2].stats(),
        ]
    }

    /// Track a new object (add to gen0).
    /// O(1) — intrusive linked list push_front, no hashing.
    ///
    /// # Safety
    /// obj must be a valid pointer to a PyObject
    pub unsafe fn track_object(&self, obj: NonNull<PyObject>) {
        let obj_ref = unsafe { obj.as_ref() };
        obj_ref.set_gc_tracked();
        obj_ref.set_gc_generation(0);

        self.generation_lists[0].write().push_front(obj);
        self.generations[0].count.fetch_add(1, Ordering::SeqCst);
        self.alloc_count.fetch_add(1, Ordering::SeqCst);
    }

    /// Untrack an object (remove from GC lists).
    /// O(1) — intrusive linked list remove by node pointer.
    ///
    /// # Safety
    /// obj must be a valid pointer to a PyObject that is currently tracked.
    /// The object's memory must still be valid (pointers are read).
    pub unsafe fn untrack_object(&self, obj: NonNull<PyObject>) {
        let obj_ref = unsafe { obj.as_ref() };

        loop {
            let obj_gen = obj_ref.gc_generation();

            let (list_lock, count) = if obj_gen <= 2 {
                (
                    &self.generation_lists[obj_gen as usize]
                        as &PyRwLock<LinkedList<GcLink, PyObject>>,
                    &self.generations[obj_gen as usize].count,
                )
            } else if obj_gen == GC_PERMANENT {
                (&self.permanent_list, &self.permanent.count)
            } else {
                return; // GC_UNTRACKED or unknown — already untracked
            };

            let mut list = list_lock.write();
            // Re-check generation under lock (may have changed due to promotion)
            if obj_ref.gc_generation() != obj_gen {
                drop(list);
                continue; // Retry with the updated generation
            }
            if unsafe { list.remove(obj) }.is_some() {
                count.fetch_sub(1, Ordering::SeqCst);
                obj_ref.clear_gc_tracked();
                obj_ref.set_gc_generation(GC_UNTRACKED);
            } else {
                // Object claims to be in this generation but wasn't found in the list.
                // This indicates a bug: the object was already removed from the list
                // without updating gc_generation, or was never inserted.
                eprintln!(
                    "GC WARNING: untrack_object failed to remove obj={obj:p} from gen={obj_gen}, \
                     tracked={}, gc_gen={}",
                    obj_ref.is_gc_tracked(),
                    obj_ref.gc_generation()
                );
            }
            return;
        }
    }

    /// Get tracked objects (for gc.get_objects)
    /// If generation is None, returns all tracked objects.
    /// If generation is Some(n), returns objects in generation n only.
    pub fn get_objects(&self, generation: Option<i32>) -> Vec<PyObjectRef> {
        fn collect_from_list(
            list: &LinkedList<GcLink, PyObject>,
        ) -> impl Iterator<Item = PyObjectRef> + '_ {
            list.iter().filter_map(|obj| obj.try_to_owned())
        }

        match generation {
            None => {
                // Return all tracked objects from all generations + permanent
                let mut result = Vec::new();
                for gen_list in &self.generation_lists {
                    result.extend(collect_from_list(&gen_list.read()));
                }
                result.extend(collect_from_list(&self.permanent_list.read()));
                result
            }
            Some(g) if (0..=2).contains(&g) => {
                let guard = self.generation_lists[g as usize].read();
                collect_from_list(&guard).collect()
            }
            _ => Vec::new(),
        }
    }

    /// Check if automatic GC should run and run it if needed.
    /// Called after object allocation.
    /// Returns true if GC was run, false otherwise.
    pub fn maybe_collect(&self) -> bool {
        if !self.is_enabled() {
            return false;
        }

        // Check gen0 threshold
        let count0 = self.generations[0].count.load(Ordering::SeqCst) as u32;
        let threshold0 = self.generations[0].threshold();
        if threshold0 > 0 && count0 >= threshold0 {
            self.collect(0);
            return true;
        }

        false
    }

    /// Perform garbage collection on the given generation
    pub fn collect(&self, generation: usize) -> CollectResult {
        self.collect_inner(generation, false)
    }

    /// Force collection even if GC is disabled (for manual gc.collect() calls)
    pub fn collect_force(&self, generation: usize) -> CollectResult {
        self.collect_inner(generation, true)
    }

    fn collect_inner(&self, generation: usize, force: bool) -> CollectResult {
        if !force && !self.is_enabled() {
            return CollectResult::default();
        }

        // Try to acquire the collecting lock
        let Some(_guard) = self.collecting.try_lock() else {
            return CollectResult::default();
        };

        #[cfg(not(target_arch = "wasm32"))]
        let start_time = std::time::Instant::now();
        #[cfg(target_arch = "wasm32")]
        let start_time = ();

        // Memory barrier to ensure visibility of all reference count updates
        // from other threads before we start analyzing the object graph.
        core::sync::atomic::fence(Ordering::SeqCst);

        let generation = generation.min(2);
        let debug = self.get_debug();

        // Clear the method cache to release strong references that
        // might prevent cycle collection (_PyType_ClearCache).
        crate::builtins::type_::type_cache_clear();

        // Step 1: Gather objects from generations 0..=generation
        // Hold read locks for the entire scan to prevent concurrent modifications.
        let gen_locks: Vec<_> = (0..=generation)
            .map(|i| self.generation_lists[i].read())
            .collect();

        let mut collecting: HashSet<GcPtr> = HashSet::new();
        for gen_list in &gen_locks {
            for obj in gen_list.iter() {
                if obj.strong_count() > 0 {
                    collecting.insert(GcPtr(NonNull::from(obj)));
                }
            }
        }

        if collecting.is_empty() {
            // Reset counts for generations whose objects were promoted away.
            // For gen2 (oldest), survivors stay in-place so don't reset gen2 count.
            let reset_end = if generation >= 2 { 2 } else { generation + 1 };
            for i in 0..reset_end {
                self.generations[i].count.store(0, Ordering::SeqCst);
            }
            let duration = elapsed_secs(&start_time);
            self.generations[generation].update_stats(0, 0, 0, duration);
            return CollectResult {
                collected: 0,
                uncollectable: 0,
                candidates: 0,
                duration,
            };
        }

        let candidates = collecting.len();

        if debug.contains(GcDebugFlags::STATS) {
            eprintln!(
                "gc: collecting {} objects from generations 0..={}",
                collecting.len(),
                generation
            );
        }

        // Step 2: Build gc_refs map (copy reference counts)
        let mut gc_refs: std::collections::HashMap<GcPtr, usize> = std::collections::HashMap::new();
        for &ptr in &collecting {
            let obj = unsafe { ptr.0.as_ref() };
            gc_refs.insert(ptr, obj.strong_count());
        }

        // Step 3: Subtract internal references
        // Pre-compute referent pointers once per object so that both step 3
        // (subtract refs) and step 4 (BFS reachability) see the same snapshot
        // of each object's children. Without this, a dict whose write lock is
        // held during one traversal but not the other can yield inconsistent
        // results, causing live objects to be incorrectly collected.
        let mut referents_map: std::collections::HashMap<GcPtr, Vec<NonNull<PyObject>>> =
            std::collections::HashMap::new();
        for &ptr in &collecting {
            let obj = unsafe { ptr.0.as_ref() };
            if obj.strong_count() == 0 {
                continue;
            }
            let referent_ptrs = unsafe { obj.gc_get_referent_ptrs() };
            referents_map.insert(ptr, referent_ptrs.clone());
            for child_ptr in referent_ptrs {
                let gc_ptr = GcPtr(child_ptr);
                if collecting.contains(&gc_ptr)
                    && let Some(refs) = gc_refs.get_mut(&gc_ptr)
                {
                    *refs = refs.saturating_sub(1);
                }
            }
        }

        // Step 4: Find reachable objects (gc_refs > 0) and traverse from them
        let mut reachable: HashSet<GcPtr> = HashSet::new();
        let mut worklist: Vec<GcPtr> = Vec::new();

        for (&ptr, &refs) in &gc_refs {
            if refs > 0 {
                reachable.insert(ptr);
                worklist.push(ptr);
            }
        }

        while let Some(ptr) = worklist.pop() {
            let obj = unsafe { ptr.0.as_ref() };
            if obj.is_gc_tracked() {
                // Reuse the pre-computed referent pointers from step 3.
                // For objects that were skipped in step 3 (strong_count was 0),
                // compute them now as a fallback.
                let referent_ptrs = referents_map
                    .get(&ptr)
                    .cloned()
                    .unwrap_or_else(|| unsafe { obj.gc_get_referent_ptrs() });
                for child_ptr in referent_ptrs {
                    let gc_ptr = GcPtr(child_ptr);
                    if collecting.contains(&gc_ptr) && reachable.insert(gc_ptr) {
                        worklist.push(gc_ptr);
                    }
                }
            }
        }

        // Step 5: Find unreachable objects
        let unreachable: Vec<GcPtr> = collecting.difference(&reachable).copied().collect();

        if debug.contains(GcDebugFlags::STATS) {
            eprintln!(
                "gc: {} reachable, {} unreachable",
                reachable.len(),
                unreachable.len()
            );
        }

        // Create strong references while read locks are still held.
        // After dropping gen_locks, other threads can untrack+free objects,
        // making the raw pointers in `reachable`/`unreachable` dangling.
        // Strong refs keep objects alive for later phases.
        //
        // Use try_to_owned() (CAS-based) instead of strong_count()+to_owned()
        // to prevent a TOCTOU race: another thread can dec() the count to 0
        // between the check and the increment, causing a use-after-free when
        // the destroying thread eventually frees the memory.
        let survivor_refs: Vec<PyObjectRef> = reachable
            .iter()
            .filter_map(|ptr| {
                let obj = unsafe { ptr.0.as_ref() };
                obj.try_to_owned()
            })
            .collect();

        let unreachable_refs: Vec<crate::PyObjectRef> = unreachable
            .iter()
            .filter_map(|ptr| {
                let obj = unsafe { ptr.0.as_ref() };
                obj.try_to_owned()
            })
            .collect();

        if unreachable.is_empty() {
            drop(gen_locks);
            self.promote_survivors(generation, &survivor_refs);
            let reset_end = if generation >= 2 { 2 } else { generation + 1 };
            for i in 0..reset_end {
                self.generations[i].count.store(0, Ordering::SeqCst);
            }
            let duration = elapsed_secs(&start_time);
            self.generations[generation].update_stats(0, 0, candidates, duration);
            return CollectResult {
                collected: 0,
                uncollectable: 0,
                candidates,
                duration,
            };
        }

        // Release read locks before finalization phase.
        drop(gen_locks);

        // Step 6: Finalize unreachable objects and handle resurrection

        if unreachable_refs.is_empty() {
            self.promote_survivors(generation, &survivor_refs);
            let reset_end = if generation >= 2 { 2 } else { generation + 1 };
            for i in 0..reset_end {
                self.generations[i].count.store(0, Ordering::SeqCst);
            }
            let duration = elapsed_secs(&start_time);
            self.generations[generation].update_stats(0, 0, candidates, duration);
            return CollectResult {
                collected: 0,
                uncollectable: 0,
                candidates,
                duration,
            };
        }

        // 6b: Record initial strong counts (for resurrection detection)
        let initial_counts: std::collections::HashMap<GcPtr, usize> = unreachable_refs
            .iter()
            .map(|obj| {
                let ptr = GcPtr(core::ptr::NonNull::from(obj.as_ref()));
                (ptr, obj.strong_count())
            })
            .collect();

        // 6c: Clear existing weakrefs BEFORE calling __del__
        let mut all_callbacks: Vec<(crate::PyRef<crate::object::PyWeak>, crate::PyObjectRef)> =
            Vec::new();
        for obj_ref in &unreachable_refs {
            let callbacks = obj_ref.gc_clear_weakrefs_collect_callbacks();
            all_callbacks.extend(callbacks);
        }
        for (wr, cb) in all_callbacks {
            if let Some(Err(e)) = crate::vm::thread::with_vm(&cb, |vm| cb.call((wr.clone(),), vm)) {
                crate::vm::thread::with_vm(&cb, |vm| {
                    vm.run_unraisable(e.clone(), Some("weakref callback".to_owned()), cb.clone());
                });
            }
        }

        // 6d: Call __del__ on unreachable objects (skip already-finalized).
        // try_call_finalizer() internally checks gc_finalized() and sets it,
        // so we must NOT set it beforehand.
        for obj_ref in &unreachable_refs {
            obj_ref.try_call_finalizer();
        }

        // Detect resurrection
        let mut resurrected_set: HashSet<GcPtr> = HashSet::new();
        let unreachable_set: HashSet<GcPtr> = unreachable.iter().copied().collect();

        for obj in &unreachable_refs {
            let ptr = GcPtr(core::ptr::NonNull::from(obj.as_ref()));
            let initial = initial_counts.get(&ptr).copied().unwrap_or(1);
            if obj.strong_count() > initial {
                resurrected_set.insert(ptr);
            }
        }

        // Transitive resurrection
        let mut worklist: Vec<GcPtr> = resurrected_set.iter().copied().collect();
        while let Some(ptr) = worklist.pop() {
            let obj = unsafe { ptr.0.as_ref() };
            let referent_ptrs = unsafe { obj.gc_get_referent_ptrs() };
            for child_ptr in referent_ptrs {
                let child_gc_ptr = GcPtr(child_ptr);
                if unreachable_set.contains(&child_gc_ptr) && resurrected_set.insert(child_gc_ptr) {
                    worklist.push(child_gc_ptr);
                }
            }
        }

        // Partition into resurrected and truly dead
        let (resurrected, truly_dead): (Vec<_>, Vec<_>) =
            unreachable_refs.into_iter().partition(|obj| {
                let ptr = GcPtr(core::ptr::NonNull::from(obj.as_ref()));
                resurrected_set.contains(&ptr)
            });

        if debug.contains(GcDebugFlags::STATS) {
            eprintln!(
                "gc: {} resurrected, {} truly dead",
                resurrected.len(),
                truly_dead.len()
            );
        }

        // Compute collected count (exclude instance dicts in truly_dead)
        let collected = {
            let dead_ptrs: HashSet<usize> = truly_dead
                .iter()
                .map(|obj| obj.as_ref() as *const PyObject as usize)
                .collect();
            let instance_dict_count = truly_dead
                .iter()
                .filter(|obj| {
                    if let Some(dict_ref) = obj.dict() {
                        dead_ptrs.contains(&(dict_ref.as_object() as *const PyObject as usize))
                    } else {
                        false
                    }
                })
                .count();
            truly_dead.len() - instance_dict_count
        };

        // Promote survivors to next generation BEFORE tp_clear.
        // move_legacy_finalizer_reachable → delete_garbage order ensures
        // survivor_refs are dropped before tp_clear, so reachable objects
        // aren't kept alive beyond the deferred-drop phase.
        self.promote_survivors(generation, &survivor_refs);
        drop(survivor_refs);

        // Resurrected objects stay tracked — just drop our references
        drop(resurrected);

        if debug.contains(GcDebugFlags::COLLECTABLE) {
            for obj in &truly_dead {
                eprintln!(
                    "gc: collectable <{} {:p}>",
                    obj.class().name(),
                    obj.as_ref()
                );
            }
        }

        if debug.contains(GcDebugFlags::SAVEALL) {
            let mut garbage_guard = self.garbage.lock();
            for obj_ref in truly_dead.iter() {
                garbage_guard.push(obj_ref.clone());
            }
        }

        if !truly_dead.is_empty() {
            // Break cycles by clearing references (tp_clear)
            // Use deferred drop context to prevent stack overflow.
            rustpython_common::refcount::with_deferred_drops(|| {
                for obj_ref in truly_dead.iter() {
                    if obj_ref.gc_has_clear() {
                        let edges = unsafe { obj_ref.gc_clear() };
                        drop(edges);
                    }
                }
                drop(truly_dead);
            });
        }

        // Reset counts for generations whose objects were promoted away.
        // For gen2 (oldest), survivors stay in-place so don't reset gen2 count.
        let reset_end = if generation >= 2 { 2 } else { generation + 1 };
        for i in 0..reset_end {
            self.generations[i].count.store(0, Ordering::SeqCst);
        }

        let duration = elapsed_secs(&start_time);
        self.generations[generation].update_stats(collected, 0, candidates, duration);

        CollectResult {
            collected,
            uncollectable: 0,
            candidates,
            duration,
        }
    }

    /// Promote surviving objects to the next generation.
    ///
    /// `survivors` must be strong references (`PyObjectRef`) to keep objects alive,
    /// since the generation read locks are released before this is called.
    ///
    /// Holds both source and destination list locks simultaneously to prevent
    /// a race where concurrent `untrack_object` reads a stale `gc_generation`
    /// and operates on the wrong list.
    fn promote_survivors(&self, from_gen: usize, survivors: &[PyObjectRef]) {
        if from_gen >= 2 {
            return; // Already in oldest generation
        }

        let next_gen = from_gen + 1;

        for obj_ref in survivors {
            let obj = obj_ref.as_ref();
            let ptr = NonNull::from(obj);
            let obj_gen = obj.gc_generation();
            if obj_gen as usize <= from_gen && obj_gen <= 2 {
                let src_gen = obj_gen as usize;

                // Lock both source and destination lists simultaneously.
                // Always ascending order (src_gen < next_gen) → no deadlock.
                let mut src = self.generation_lists[src_gen].write();
                let mut dst = self.generation_lists[next_gen].write();

                // Re-check under locks: object might have been untracked concurrently
                if obj.gc_generation() != obj_gen || !obj.is_gc_tracked() {
                    continue;
                }

                if unsafe { src.remove(ptr) }.is_some() {
                    self.generations[src_gen]
                        .count
                        .fetch_sub(1, Ordering::SeqCst);

                    dst.push_front(ptr);
                    self.generations[next_gen]
                        .count
                        .fetch_add(1, Ordering::SeqCst);

                    obj.set_gc_generation(next_gen as u8);
                }
            }
        }
    }

    /// Get count of frozen objects
    pub fn get_freeze_count(&self) -> usize {
        self.permanent.count()
    }

    /// Freeze all tracked objects (move to permanent generation).
    /// Lock order: generation_lists[i] → permanent_list (consistent with unfreeze).
    pub fn freeze(&self) {
        let mut count = 0usize;

        for (gen_idx, gen_list) in self.generation_lists.iter().enumerate() {
            let mut list = gen_list.write();
            let mut perm = self.permanent_list.write();
            while let Some(ptr) = list.pop_front() {
                perm.push_front(ptr);
                unsafe { ptr.as_ref().set_gc_generation(GC_PERMANENT) };
                count += 1;
            }
            self.generations[gen_idx].count.store(0, Ordering::SeqCst);
        }

        self.permanent.count.fetch_add(count, Ordering::SeqCst);
    }

    /// Unfreeze all objects (move from permanent to gen2).
    /// Lock order: generation_lists[2] → permanent_list (consistent with freeze).
    pub fn unfreeze(&self) {
        let mut count = 0usize;

        {
            let mut gen2 = self.generation_lists[2].write();
            let mut perm_list = self.permanent_list.write();
            while let Some(ptr) = perm_list.pop_front() {
                gen2.push_front(ptr);
                unsafe { ptr.as_ref().set_gc_generation(2) };
                count += 1;
            }
            self.permanent.count.store(0, Ordering::SeqCst);
        }

        self.generations[2].count.fetch_add(count, Ordering::SeqCst);
    }

    /// Reset all locks to unlocked state after fork().
    ///
    /// After fork(), only the forking thread survives. Any lock held by another
    /// thread is permanently stuck. This resets them by zeroing the raw bytes.
    ///
    /// # Safety
    /// Must only be called after fork() in the child process when no other
    /// threads exist. The calling thread must NOT hold any of these locks.
    #[cfg(all(unix, feature = "threading"))]
    pub unsafe fn reinit_after_fork(&self) {
        use crate::common::lock::{reinit_mutex_after_fork, reinit_rwlock_after_fork};

        unsafe {
            reinit_mutex_after_fork(&self.collecting);
            reinit_mutex_after_fork(&self.garbage);
            reinit_mutex_after_fork(&self.callbacks);

            for generation in &self.generations {
                generation.reinit_stats_after_fork();
            }
            self.permanent.reinit_stats_after_fork();

            for rw in &self.generation_lists {
                reinit_rwlock_after_fork(rw);
            }
            reinit_rwlock_after_fork(&self.permanent_list);
        }
    }
}

/// Get a reference to the GC state.
///
/// In threading mode this is a true global (OnceLock).
/// In non-threading mode this is thread-local, because PyRwLock/PyMutex
/// use Cell-based locks that are not Sync.
pub fn gc_state() -> &'static GcState {
    rustpython_common::static_cell! {
        static GC_STATE: GcState;
    }
    GC_STATE.get_or_init(GcState::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gc_state_default() {
        let state = GcState::new();
        assert!(state.is_enabled());
        assert_eq!(state.get_debug(), GcDebugFlags::empty());
        assert_eq!(state.get_threshold(), (2000, 10, 0));
        assert_eq!(state.get_count(), (0, 0, 0));
    }

    #[test]
    fn test_gc_enable_disable() {
        let state = GcState::new();
        assert!(state.is_enabled());
        state.disable();
        assert!(!state.is_enabled());
        state.enable();
        assert!(state.is_enabled());
    }

    #[test]
    fn test_gc_threshold() {
        let state = GcState::new();
        state.set_threshold(100, Some(20), Some(30));
        assert_eq!(state.get_threshold(), (100, 20, 30));
    }

    #[test]
    fn test_gc_debug_flags() {
        let state = GcState::new();
        state.set_debug(GcDebugFlags::STATS | GcDebugFlags::COLLECTABLE);
        assert_eq!(
            state.get_debug(),
            GcDebugFlags::STATS | GcDebugFlags::COLLECTABLE
        );
    }
}
