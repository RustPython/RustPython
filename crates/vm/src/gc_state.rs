//! Garbage Collection State and Algorithm
//!
//! This module implements CPython-compatible generational garbage collection
//! for RustPython, using an intrusive doubly-linked list approach.

use crate::common::lock::PyMutex;
use crate::{PyObject, PyObjectRef};
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
#[derive(Debug, Default, Clone, Copy)]
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
    /// Mutex for collection (prevents concurrent collections).
    /// Used by collect_inner when the actual collection algorithm is enabled.
    #[allow(dead_code)]
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
    /// Currently a stub — returns false.
    pub fn maybe_collect(&self) -> bool {
        false
    }

    /// Perform garbage collection on the given generation.
    /// Returns (collected_count, uncollectable_count).
    ///
    /// Currently a stub — the actual collection algorithm requires EBR
    /// and will be added in a follow-up.
    pub fn collect(&self, _generation: usize) -> (usize, usize) {
        (0, 0)
    }

    /// Force collection even if GC is disabled (for manual gc.collect() calls).
    /// Currently a stub.
    pub fn collect_force(&self, _generation: usize) -> (usize, usize) {
        (0, 0)
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
