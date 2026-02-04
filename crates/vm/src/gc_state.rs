//! Garbage Collection State and Algorithm
//!
//! This module implements CPython-compatible generational garbage collection
//! for RustPython, using an intrusive doubly-linked list approach.

use crate::common::lock::PyMutex;
use crate::{AsObject, PyObject, PyObjectRef};
use core::ptr::NonNull;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::collections::HashSet;
use std::sync::{Mutex, RwLock};

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

/// Statistics for a single generation (gc_generation_stats)
#[derive(Debug, Default)]
pub struct GcStats {
    pub collections: usize,
    pub collected: usize,
    pub uncollectable: usize,
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
    pub const fn new(threshold: u32) -> Self {
        Self {
            count: AtomicUsize::new(0),
            threshold: AtomicU32::new(threshold),
            stats: PyMutex::new(GcStats {
                collections: 0,
                collected: 0,
                uncollectable: 0,
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
        }
    }

    pub fn update_stats(&self, collected: usize, uncollectable: usize) {
        let mut guard = self.stats.lock();
        guard.collections += 1;
        guard.collected += collected;
        guard.uncollectable += uncollectable;
    }
}

/// Wrapper for raw pointer to make it Send + Sync
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct GcObjectPtr(NonNull<PyObject>);

// SAFETY: We only use this for tracking objects, and proper synchronization is used
unsafe impl Send for GcObjectPtr {}
unsafe impl Sync for GcObjectPtr {}

/// Global GC state
pub struct GcState {
    /// 3 generations (0 = youngest, 2 = oldest)
    pub generations: [GcGeneration; 3],
    /// Permanent generation (frozen objects)
    pub permanent: GcGeneration,
    /// GC enabled flag
    pub enabled: AtomicBool,
    /// Per-generation object tracking (for correct gc_refs algorithm)
    /// Objects start in gen0, survivors move to gen1, then gen2
    generation_objects: [RwLock<HashSet<GcObjectPtr>>; 3],
    /// Frozen/permanent objects (excluded from normal GC)
    permanent_objects: RwLock<HashSet<GcObjectPtr>>,
    /// Debug flags
    pub debug: AtomicU32,
    /// gc.garbage list (uncollectable objects with __del__)
    pub garbage: PyMutex<Vec<PyObjectRef>>,
    /// gc.callbacks list
    pub callbacks: PyMutex<Vec<PyObjectRef>>,
    /// Mutex for collection (prevents concurrent collections)
    collecting: Mutex<()>,
    /// Allocation counter for gen0
    alloc_count: AtomicUsize,
    /// Registry of all tracked objects (for cycle detection)
    tracked_objects: RwLock<HashSet<GcObjectPtr>>,
    /// Objects that have been finalized (__del__ already called)
    /// Prevents calling __del__ multiple times on resurrected objects
    finalized_objects: RwLock<HashSet<GcObjectPtr>>,
}

// SAFETY: All fields are either inherently Send/Sync (atomics, RwLock, Mutex) or protected by PyMutex.
// PyMutex<Vec<PyObjectRef>> is safe to share/send across threads because access is synchronized.
// PyObjectRef itself is Send, and interior mutability is guarded by the mutex.
unsafe impl Send for GcState {}
unsafe impl Sync for GcState {}

impl Default for GcState {
    fn default() -> Self {
        Self::new()
    }
}

impl GcState {
    pub fn new() -> Self {
        Self {
            generations: [
                GcGeneration::new(2000), // young
                GcGeneration::new(10),   // old[0]
                GcGeneration::new(0),    // old[1]
            ],
            permanent: GcGeneration::new(0),
            enabled: AtomicBool::new(true),
            generation_objects: [
                RwLock::new(HashSet::new()),
                RwLock::new(HashSet::new()),
                RwLock::new(HashSet::new()),
            ],
            permanent_objects: RwLock::new(HashSet::new()),
            debug: AtomicU32::new(0),
            garbage: PyMutex::new(Vec::new()),
            callbacks: PyMutex::new(Vec::new()),
            collecting: Mutex::new(()),
            alloc_count: AtomicUsize::new(0),
            tracked_objects: RwLock::new(HashSet::new()),
            finalized_objects: RwLock::new(HashSet::new()),
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

    /// Track a new object (add to gen0)
    /// Called when IS_TRACE objects are created
    ///
    /// # Safety
    /// obj must be a valid pointer to a PyObject
    pub unsafe fn track_object(&self, obj: NonNull<PyObject>) {
        let gc_ptr = GcObjectPtr(obj);

        // _PyObject_GC_TRACK
        let obj_ref = unsafe { obj.as_ref() };
        obj_ref.set_gc_tracked();

        // Add to generation 0 tracking first (for correct gc_refs algorithm)
        // Only increment count if we successfully add to the set
        if let Ok(mut gen0) = self.generation_objects[0].write()
            && gen0.insert(gc_ptr)
        {
            self.generations[0].count.fetch_add(1, Ordering::SeqCst);
            self.alloc_count.fetch_add(1, Ordering::SeqCst);
        }

        // Also add to global tracking (for get_objects, etc.)
        if let Ok(mut tracked) = self.tracked_objects.write() {
            tracked.insert(gc_ptr);
        }
    }

    /// Untrack an object (remove from GC lists)
    /// Called when objects are deallocated
    ///
    /// # Safety
    /// obj must be a valid pointer to a PyObject
    pub unsafe fn untrack_object(&self, obj: NonNull<PyObject>) {
        let gc_ptr = GcObjectPtr(obj);

        // Remove from generation tracking lists and decrement the correct generation's count
        for (gen_idx, generation) in self.generation_objects.iter().enumerate() {
            if let Ok(mut gen_set) = generation.write()
                && gen_set.remove(&gc_ptr)
            {
                // Decrement count for the generation we removed from
                let count = self.generations[gen_idx].count.load(Ordering::SeqCst);
                if count > 0 {
                    self.generations[gen_idx]
                        .count
                        .fetch_sub(1, Ordering::SeqCst);
                }
                break; // Object can only be in one generation
            }
        }

        // Remove from global tracking
        if let Ok(mut tracked) = self.tracked_objects.write() {
            tracked.remove(&gc_ptr);
        }

        // Remove from permanent tracking
        if let Ok(mut permanent) = self.permanent_objects.write()
            && permanent.remove(&gc_ptr)
        {
            let count = self.permanent.count.load(Ordering::SeqCst);
            if count > 0 {
                self.permanent.count.fetch_sub(1, Ordering::SeqCst);
            }
        }

        // Remove from finalized set
        if let Ok(mut finalized) = self.finalized_objects.write() {
            finalized.remove(&gc_ptr);
        }
    }

    /// Check if an object has been finalized
    pub fn is_finalized(&self, obj: NonNull<PyObject>) -> bool {
        let gc_ptr = GcObjectPtr(obj);
        if let Ok(finalized) = self.finalized_objects.read() {
            finalized.contains(&gc_ptr)
        } else {
            false
        }
    }

    /// Mark an object as finalized
    pub fn mark_finalized(&self, obj: NonNull<PyObject>) {
        let gc_ptr = GcObjectPtr(obj);
        if let Ok(mut finalized) = self.finalized_objects.write() {
            finalized.insert(gc_ptr);
        }
    }

    /// Get tracked objects (for gc.get_objects)
    /// If generation is None, returns all tracked objects.
    /// If generation is Some(n), returns objects in generation n only.
    pub fn get_objects(&self, generation: Option<i32>) -> Vec<PyObjectRef> {
        match generation {
            None => {
                // Return all tracked objects
                if let Ok(tracked) = self.tracked_objects.read() {
                    tracked
                        .iter()
                        .filter_map(|ptr| {
                            let obj = unsafe { ptr.0.as_ref() };
                            if obj.strong_count() > 0 {
                                Some(obj.to_owned())
                            } else {
                                None
                            }
                        })
                        .collect()
                } else {
                    Vec::new()
                }
            }
            Some(g) if (0..=2).contains(&g) => {
                // Return objects in specific generation
                let gen_idx = g as usize;
                if let Ok(gen_set) = self.generation_objects[gen_idx].read() {
                    gen_set
                        .iter()
                        .filter_map(|ptr| {
                            let obj = unsafe { ptr.0.as_ref() };
                            if obj.strong_count() > 0 {
                                Some(obj.to_owned())
                            } else {
                                None
                            }
                        })
                        .collect()
                } else {
                    Vec::new()
                }
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

        // _PyObject_GC_Alloc checks thresholds

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
    /// Returns (collected_count, uncollectable_count)
    ///
    /// Implements CPython-compatible generational GC algorithm:
    /// - Only collects objects from generations 0 to `generation`
    /// - Uses gc_refs algorithm: gc_refs = strong_count - internal_refs
    /// - Only subtracts references between objects IN THE SAME COLLECTION
    ///
    /// If `force` is true, collection runs even if GC is disabled (for manual gc.collect() calls)
    pub fn collect(&self, generation: usize) -> (usize, usize) {
        self.collect_inner(generation, false)
    }

    /// Force collection even if GC is disabled (for manual gc.collect() calls)
    pub fn collect_force(&self, generation: usize) -> (usize, usize) {
        self.collect_inner(generation, true)
    }

    fn collect_inner(&self, generation: usize, force: bool) -> (usize, usize) {
        if !force && !self.is_enabled() {
            return (0, 0);
        }

        // Try to acquire the collecting lock
        let _guard = match self.collecting.try_lock() {
            Ok(g) => g,
            Err(_) => return (0, 0),
        };

        // Enter EBR critical section for the entire collection.
        // This ensures that any objects being freed by other threads won't have
        // their memory actually deallocated until we exit this critical section.
        // Other threads' deferred deallocations will wait for us to unpin.
        let ebr_guard = rustpython_common::epoch::pin();

        // Memory barrier to ensure visibility of all reference count updates
        // from other threads before we start analyzing the object graph.
        std::sync::atomic::fence(Ordering::SeqCst);

        let generation = generation.min(2);
        let debug = self.get_debug();

        // Step 1: Gather objects from generations 0..=generation
        // Hold read locks for the entire collection to prevent other threads
        // from untracking objects while we're iterating.
        let gen_locks: Vec<_> = (0..=generation)
            .filter_map(|i| self.generation_objects[i].read().ok())
            .collect();

        let mut collecting: HashSet<GcObjectPtr> = HashSet::new();
        for gen_set in &gen_locks {
            for &ptr in gen_set.iter() {
                let obj = unsafe { ptr.0.as_ref() };
                if obj.strong_count() > 0 {
                    collecting.insert(ptr);
                }
            }
        }

        if collecting.is_empty() {
            // Reset gen0 count even if nothing to collect
            self.generations[0].count.store(0, Ordering::SeqCst);
            self.generations[generation].update_stats(0, 0);
            return (0, 0);
        }

        if debug.contains(GcDebugFlags::STATS) {
            eprintln!(
                "gc: collecting {} objects from generations 0..={}",
                collecting.len(),
                generation
            );
        }

        // Step 2: Build gc_refs map (copy reference counts)
        let mut gc_refs: std::collections::HashMap<GcObjectPtr, usize> =
            std::collections::HashMap::new();
        for &ptr in &collecting {
            let obj = unsafe { ptr.0.as_ref() };
            gc_refs.insert(ptr, obj.strong_count());
        }

        // Step 3: Subtract internal references
        // CRITICAL: Only subtract refs to objects IN THE COLLECTING SET
        for &ptr in &collecting {
            let obj = unsafe { ptr.0.as_ref() };
            // Double-check object is still alive
            if obj.strong_count() == 0 {
                continue;
            }
            let referent_ptrs = unsafe { obj.gc_get_referent_ptrs() };
            for child_ptr in referent_ptrs {
                let gc_ptr = GcObjectPtr(child_ptr);
                // Only decrement if child is also in the collecting set!
                if collecting.contains(&gc_ptr)
                    && let Some(refs) = gc_refs.get_mut(&gc_ptr)
                {
                    *refs = refs.saturating_sub(1);
                }
            }
        }

        // Step 4: Find reachable objects (gc_refs > 0) and traverse from them
        // Objects with gc_refs > 0 are definitely reachable from outside.
        // We need to mark all objects reachable from them as also reachable.
        let mut reachable: HashSet<GcObjectPtr> = HashSet::new();
        let mut worklist: Vec<GcObjectPtr> = Vec::new();

        // Start with objects that have gc_refs > 0
        for (&ptr, &refs) in &gc_refs {
            if refs > 0 {
                reachable.insert(ptr);
                worklist.push(ptr);
            }
        }

        // Traverse reachable objects to find more reachable ones
        while let Some(ptr) = worklist.pop() {
            let obj = unsafe { ptr.0.as_ref() };
            if obj.is_gc_tracked() {
                let referent_ptrs = unsafe { obj.gc_get_referent_ptrs() };
                for child_ptr in referent_ptrs {
                    let gc_ptr = GcObjectPtr(child_ptr);
                    // If child is in collecting set and not yet marked reachable
                    if collecting.contains(&gc_ptr) && reachable.insert(gc_ptr) {
                        worklist.push(gc_ptr);
                    }
                }
            }
        }

        // Step 5: Find unreachable objects (in collecting but not in reachable)
        let unreachable: Vec<GcObjectPtr> = collecting.difference(&reachable).copied().collect();

        if debug.contains(GcDebugFlags::STATS) {
            eprintln!(
                "gc: {} reachable, {} unreachable",
                reachable.len(),
                unreachable.len()
            );
        }

        if unreachable.is_empty() {
            // No cycles found - promote survivors to next generation
            drop(gen_locks); // Release read locks before promoting
            self.promote_survivors(generation, &collecting);
            // Reset gen0 count
            self.generations[0].count.store(0, Ordering::SeqCst);
            self.generations[generation].update_stats(0, 0);
            return (0, 0);
        }

        // Release read locks before finalization phase.
        // This allows other threads to untrack objects while we finalize.
        drop(gen_locks);

        // Step 6: Finalize unreachable objects and handle resurrection

        // 6a: Get references to all unreachable objects
        let unreachable_refs: Vec<crate::PyObjectRef> = unreachable
            .iter()
            .filter_map(|ptr| {
                let obj = unsafe { ptr.0.as_ref() };
                if obj.strong_count() > 0 {
                    Some(obj.to_owned())
                } else {
                    None
                }
            })
            .collect();

        if unreachable_refs.is_empty() {
            self.promote_survivors(generation, &reachable);
            // Reset gen0 count
            self.generations[0].count.store(0, Ordering::SeqCst);
            self.generations[generation].update_stats(0, 0);
            return (0, 0);
        }

        // 6b: Record initial strong counts (for resurrection detection)
        // Each object has +1 from unreachable_refs, so initial count includes that
        let initial_counts: std::collections::HashMap<GcObjectPtr, usize> = unreachable_refs
            .iter()
            .map(|obj| {
                let ptr = GcObjectPtr(core::ptr::NonNull::from(obj.as_ref()));
                (ptr, obj.strong_count())
            })
            .collect();

        // 6c: Clear existing weakrefs BEFORE calling __del__
        // This invalidates existing weakrefs, but new weakrefs created during __del__
        // will still work (WeakRefList::add restores inner.obj if cleared)
        //
        // CRITICAL: We use a two-phase approach to match CPython behavior:
        // Phase 1: Clear ALL weakrefs (set inner.obj = None) and collect callbacks
        // Phase 2: Invoke ALL callbacks
        // This ensures that when a callback runs, ALL weakrefs to unreachable objects
        // are already dead (return None when called).
        let mut all_callbacks: Vec<(crate::PyRef<crate::object::PyWeak>, crate::PyObjectRef)> =
            Vec::new();
        for obj_ref in &unreachable_refs {
            let callbacks = obj_ref.gc_clear_weakrefs_collect_callbacks();
            all_callbacks.extend(callbacks);
        }
        // Phase 2: Now call all callbacks - at this point ALL weakrefs are cleared
        for (wr, cb) in all_callbacks {
            if let Some(Err(e)) = crate::vm::thread::with_vm(&cb, |vm| cb.call((wr.clone(),), vm)) {
                // Report the exception via run_unraisable
                crate::vm::thread::with_vm(&cb, |vm| {
                    vm.run_unraisable(e.clone(), Some("weakref callback".to_owned()), cb.clone());
                });
            }
            // If with_vm returns None, we silently skip - no VM available to handle errors
        }

        // 6d: Call __del__ on all unreachable objects
        // This allows resurrection to work correctly
        // Skip objects that have already been finalized (prevents multiple __del__ calls)
        for obj_ref in &unreachable_refs {
            let ptr = GcObjectPtr(core::ptr::NonNull::from(obj_ref.as_ref()));
            let already_finalized = if let Ok(finalized) = self.finalized_objects.read() {
                finalized.contains(&ptr)
            } else {
                false
            };

            if !already_finalized {
                // Mark as finalized BEFORE calling __del__
                // This ensures is_finalized() returns True inside __del__
                if let Ok(mut finalized) = self.finalized_objects.write() {
                    finalized.insert(ptr);
                }
                obj_ref.try_call_finalizer();
            }
        }

        // 6d: Detect resurrection - strong_count increased means object was resurrected
        // Step 1: Find directly resurrected objects (strong_count increased)
        let mut resurrected_set: HashSet<GcObjectPtr> = HashSet::new();
        let unreachable_set: HashSet<GcObjectPtr> = unreachable.iter().copied().collect();

        for obj in &unreachable_refs {
            let ptr = GcObjectPtr(core::ptr::NonNull::from(obj.as_ref()));
            let initial = initial_counts.get(&ptr).copied().unwrap_or(1);
            if obj.strong_count() > initial {
                resurrected_set.insert(ptr);
            }
        }

        // Step 2: Transitive resurrection - objects reachable from resurrected are also resurrected
        // This is critical for cases like: Lazarus resurrects itself, its cargo should also survive
        let mut worklist: Vec<GcObjectPtr> = resurrected_set.iter().copied().collect();
        while let Some(ptr) = worklist.pop() {
            let obj = unsafe { ptr.0.as_ref() };
            let referent_ptrs = unsafe { obj.gc_get_referent_ptrs() };
            for child_ptr in referent_ptrs {
                let child_gc_ptr = GcObjectPtr(child_ptr);
                // If child is in unreachable set and not yet marked as resurrected
                if unreachable_set.contains(&child_gc_ptr) && resurrected_set.insert(child_gc_ptr) {
                    worklist.push(child_gc_ptr);
                }
            }
        }

        // Step 3: Partition into resurrected and truly dead
        let (resurrected, truly_dead): (Vec<_>, Vec<_>) =
            unreachable_refs.into_iter().partition(|obj| {
                let ptr = GcObjectPtr(core::ptr::NonNull::from(obj.as_ref()));
                resurrected_set.contains(&ptr)
            });

        let resurrected_count = resurrected.len();

        if debug.contains(GcDebugFlags::STATS) {
            eprintln!(
                "gc: {} resurrected, {} truly dead",
                resurrected_count,
                truly_dead.len()
            );
        }

        // 6e: Break cycles ONLY for truly dead objects (not resurrected)
        // Compute collected count: exclude instance dicts that are also in truly_dead.
        // In CPython 3.12+, instance dicts are managed inline and not separately tracked,
        // so they don't count toward the collected total.
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

        // 6e-1: If DEBUG_SAVEALL is set, save truly dead objects to garbage
        if debug.contains(GcDebugFlags::SAVEALL) {
            let mut garbage_guard = self.garbage.lock();
            for obj_ref in truly_dead.iter() {
                garbage_guard.push(obj_ref.clone());
            }
        }

        if !truly_dead.is_empty() {
            // 6g: Break cycles by clearing references (tp_clear)
            // Weakrefs were already cleared in step 6c, but new weakrefs created
            // during __del__ (step 6d) can still be upgraded.
            //
            // Clear and destroy objects using the ebr_guard from the start of collection.
            // The guard ensures deferred deallocations from other threads wait for us.
            rustpython_common::refcount::with_deferred_drops(|| {
                for obj_ref in truly_dead.iter() {
                    if obj_ref.gc_has_clear() {
                        let edges = unsafe { obj_ref.gc_clear() };
                        drop(edges);
                    }
                }
                // Drop truly_dead references, triggering actual deallocation
                drop(truly_dead);
            });
        }

        // 6f: Resurrected objects stay in tracked_objects (they're still alive)
        // Just drop our references to them
        drop(resurrected);

        // Promote survivors (reachable objects) to next generation
        self.promote_survivors(generation, &reachable);

        // Reset gen0 count after collection (enables automatic GC to trigger again)
        self.generations[0].count.store(0, Ordering::SeqCst);

        self.generations[generation].update_stats(collected, 0);

        // Flush EBR deferred operations before exiting collection.
        // This ensures any deferred deallocations from this collection are executed.
        ebr_guard.flush();

        (collected, 0)
    }

    /// Promote surviving objects to the next generation
    fn promote_survivors(&self, from_gen: usize, survivors: &HashSet<GcObjectPtr>) {
        if from_gen >= 2 {
            return; // Already in oldest generation
        }

        let next_gen = from_gen + 1;

        for &ptr in survivors {
            // Remove from current generation
            for gen_idx in 0..=from_gen {
                if let Ok(mut gen_set) = self.generation_objects[gen_idx].write()
                    && gen_set.remove(&ptr)
                {
                    // Decrement count for source generation
                    let count = self.generations[gen_idx].count.load(Ordering::SeqCst);
                    if count > 0 {
                        self.generations[gen_idx]
                            .count
                            .fetch_sub(1, Ordering::SeqCst);
                    }

                    // Add to next generation
                    if let Ok(mut next_set) = self.generation_objects[next_gen].write()
                        && next_set.insert(ptr)
                    {
                        // Increment count for target generation
                        self.generations[next_gen]
                            .count
                            .fetch_add(1, Ordering::SeqCst);
                    }
                    break;
                }
            }
        }
    }

    /// Get count of frozen objects
    pub fn get_freeze_count(&self) -> usize {
        self.permanent.count()
    }

    /// Freeze all tracked objects (move to permanent generation)
    pub fn freeze(&self) {
        // Move all objects from gen0-2 to permanent
        let mut objects_to_freeze: Vec<GcObjectPtr> = Vec::new();

        for (gen_idx, generation) in self.generation_objects.iter().enumerate() {
            if let Ok(mut gen_set) = generation.write() {
                objects_to_freeze.extend(gen_set.drain());
                self.generations[gen_idx].count.store(0, Ordering::SeqCst);
            }
        }

        // Add to permanent set
        if let Ok(mut permanent) = self.permanent_objects.write() {
            let count = objects_to_freeze.len();
            for ptr in objects_to_freeze {
                permanent.insert(ptr);
            }
            self.permanent.count.fetch_add(count, Ordering::SeqCst);
        }
    }

    /// Unfreeze all objects (move from permanent to gen2)
    pub fn unfreeze(&self) {
        let mut objects_to_unfreeze: Vec<GcObjectPtr> = Vec::new();

        if let Ok(mut permanent) = self.permanent_objects.write() {
            objects_to_unfreeze.extend(permanent.drain());
            self.permanent.count.store(0, Ordering::SeqCst);
        }

        // Add to generation 2
        if let Ok(mut gen2) = self.generation_objects[2].write() {
            let count = objects_to_unfreeze.len();
            for ptr in objects_to_unfreeze {
                gen2.insert(ptr);
            }
            self.generations[2].count.fetch_add(count, Ordering::SeqCst);
        }
    }
}

use std::sync::OnceLock;

/// Global GC state instance
/// Using a static because GC needs to be accessible from object allocation/deallocation
static GC_STATE: OnceLock<GcState> = OnceLock::new();

/// Get a reference to the global GC state
pub fn gc_state() -> &'static GcState {
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
