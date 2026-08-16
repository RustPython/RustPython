//! Garbage Collection State and Algorithm
//!
//! Generational garbage collection using an intrusive doubly-linked list.

use crate::common::linked_list::LinkedList;
use crate::common::lock::{PyMutex, PyRwLock};
use crate::object::{GC_NO_OWNER, GC_PERMANENT, GC_UNTRACKED, GcLink, GcOwner};
use crate::{AsObject, PyObject, PyObjectRef};
use core::ptr::NonNull;
use core::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, AtomicUsize, Ordering};

fn elapsed_secs(
    #[cfg(target_arch = "wasm32")] _start: (),
    #[cfg(not(target_arch = "wasm32"))] start: std::time::Instant,
) -> f64 {
    cfg_select! {
        target_arch = "wasm32" => 0.0,
        _ => start.elapsed().as_secs_f64(),
    }
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
#[derive(Clone, Copy, Debug, Default)]
pub struct CollectResult {
    pub collected: usize,
    pub uncollectable: usize,
    pub candidates: usize,
    pub duration: f64,
}

/// Statistics for a single generation (gc_generation_stats)
#[derive(Clone, Copy, Debug, Default)]
pub struct GcStats {
    pub collections: usize,
    pub collected: usize,
    pub uncollectable: usize,
    pub candidates: usize,
    pub duration: f64,
}

/// One generation's collection policy and statistics, per interpreter.
///
/// The objects themselves live in the process-wide lists on [`GcState`], so the
/// occupancy count sits there; what an interpreter owns is when to collect and
/// what its own collections have done.
pub struct GcGeneration {
    /// Threshold for triggering collection
    threshold: AtomicU32,
    /// Collection statistics
    stats: PyMutex<GcStats>,
}

impl GcGeneration {
    #[must_use]
    pub const fn new(threshold: u32) -> Self {
        Self {
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

    /// Relaxed: this is policy read once per allocation, and a collection
    /// racing `gc.set_threshold()` may use either value.
    pub fn threshold(&self) -> u32 {
        self.threshold.load(Ordering::Relaxed)
    }

    pub fn set_threshold(&self, value: u32) {
        self.threshold.store(value, Ordering::Relaxed);
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

/// Drop one from a generation's occupancy.
///
/// A collection resets the counts of the generations it emptied, but it only
/// empties its own interpreter's objects; another interpreter's stay behind with
/// the count already zeroed, and untracking one of those must not wrap.
fn release_count(count: &AtomicUsize) {
    if count.load(Ordering::Relaxed) > 0 {
        count.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Whether `owner`'s collections act on `obj`.
///
/// Objects with no owner — everything the shared context allocates, and anything
/// allocated with no interpreter current — belong to all of them.
fn is_owned_by(obj: &PyObject, owner: GcOwner) -> bool {
    let obj_owner = obj.gc_owner();
    obj_owner == owner || obj_owner == GC_NO_OWNER
}

/// Wrapper for NonNull<PyObject> to impl Hash/Eq for use in temporary collection sets.
/// Only used within collect_inner, never shared across threads.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct GcPtr(NonNull<PyObject>);

/// Hashing for the tables a collection keys by an object's address.
///
/// The default hasher is SipHash, which buys resistance against a caller
/// choosing keys that collide. Nothing chooses these keys: they are addresses
/// this process handed out, and the tables live and die inside one collection.
/// What a collection needs from them is speed -- it hashes every tracked
/// object and every edge between them -- so this runs the address through a
/// handful of multiplies and shifts instead. The shifts are what earns the
/// speed: a table picks its bucket from the low bits, and an address arrives
/// with its low bits zeroed by alignment, so entropy has to be carried
/// downward or every object lands in the same few buckets.
#[derive(Default)]
struct GcPtrHasher(u64);

impl core::hash::Hasher for GcPtrHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write_usize(&mut self, value: usize) {
        let mut z = (value as u64).wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        self.0 = z ^ (z >> 31);
    }

    fn write(&mut self, bytes: &[u8]) {
        // Addresses reach this hasher through `write_usize`; a key hashed any
        // other way still has to land somewhere sensible.
        for &byte in bytes {
            self.0 = (self.0 ^ u64::from(byte)).wrapping_mul(0x0100_0000_01B3);
        }
    }
}

type GcBuildHasher = core::hash::BuildHasherDefault<GcPtrHasher>;
type GcSet<T> = std::collections::HashSet<T, GcBuildHasher>;
type GcMap<K, V> = std::collections::HashMap<K, V, GcBuildHasher>;

/// RAII barrier that parks every other thread for the pointer-reading phases
/// of a collection and lets them run again before finalizers execute.
///
/// Reference subtraction, the reachability walk and the strong-reference
/// snapshot dereference the interpreter state of every tracked object,
/// including the `localsplus` of frames that other threads are actively
/// executing. Those writes carry no synchronization, so the reads are only
/// well-defined while all other threads are parked at a safepoint. Restarting
/// happens explicitly once the snapshot has pinned every object; `Drop` is a
/// backstop that also restarts on the early-return paths.
///
/// A collection acts on one interpreter's objects, but its candidates include
/// the ones no interpreter owns, which every interpreter can reference and so
/// incref. Reading a refcount that another interpreter is changing is what
/// makes an object look unreachable when it is not, so every live interpreter
/// is stopped, not just the collecting one. Stopping in `runtime` id order
/// keeps exclusion acquisition ordered; the `collecting` mutex additionally
/// serializes collections process-wide, so no second collector can take these
/// exclusions in another order.
#[cfg(feature = "threading")]
struct CollectStopTheWorld {
    /// Stopped interpreter states, in stop order. Held as strong references so
    /// an interpreter cannot be dropped between stop and restart, and kept past
    /// the restart so that releasing the last one — which frees that
    /// interpreter's objects, and so removes them from these lists — happens
    /// after the collection has let go of the generation locks.
    stopped: Vec<crate::common::rc::PyRc<crate::vm::PyGlobalState>>,
    /// Keeps interpreters from registering between the snapshot below and the
    /// restart. One registered in that window would be missing from `stopped`,
    /// so its bootstrap would keep running — and mutating the shared generation
    /// lists — while this collection reads them.
    admission: Option<parking_lot::MutexGuard<'static, ()>>,
    restarted: bool,
}

#[cfg(feature = "threading")]
impl CollectStopTheWorld {
    /// Request stop-the-world on every live interpreter when the current thread
    /// has an attached VM. Falls back to no barrier when no VM is attached (the
    /// tracked-object reads then run without other threads only if the caller
    /// guarantees it).
    fn new() -> Self {
        // No attached VM means no interpreter is running Python on this thread;
        // keep the historical no-barrier fallback.
        if !crate::vm::thread::current_vm_is_set() {
            return Self {
                stopped: Vec::new(),
                admission: None,
                restarted: true,
            };
        }

        // Accumulate into a live `Self` rather than a bare Vec: if a later
        // `stop_the_world` unwinds, dropping this guard restarts the
        // interpreters already stopped, instead of leaving their threads parked
        // and their exclusion held forever.
        let mut guard = Self {
            stopped: Vec::new(),
            admission: Some(crate::vm::runtime::lock_admission_for_stop()),
            restarted: false,
        };
        for state in crate::vm::runtime::live_interpreter_states() {
            state.stop_the_world.stop_the_world(&state);
            guard.stopped.push(state);
        }
        guard
    }

    /// Restart the world. Idempotent.
    fn restart(&mut self) {
        if self.restarted {
            return;
        }
        self.restarted = true;
        // Reverse of the stop order. The references stay until this guard is
        // dropped; see the field comment.
        for state in self.stopped.iter().rev() {
            state.stop_the_world.start_the_world(state);
        }
        // Nothing is parked any more, so registration may resume.
        self.admission = None;
    }

    /// Whether this collection actually stopped the world.
    #[cfg(all(unix, debug_assertions))]
    fn is_stopped(&self) -> bool {
        !self.stopped.is_empty()
    }
}

#[cfg(feature = "threading")]
impl Drop for CollectStopTheWorld {
    fn drop(&mut self) {
        self.restart();
    }
}

/// The process-wide object lists every interpreter's collections walk.
///
/// Interpreter-owned policy and results live in [`GcInterpreterState`]; what is
/// here is shared because the lists are: an object is untracked from
/// `default_dealloc`, where no interpreter is in scope, so it has to be findable
/// without one.
pub struct GcState {
    /// Per-generation intrusive linked lists for object tracking.
    /// Objects start in gen0, survivors are promoted to gen1, then gen2.
    generation_lists: [PyRwLock<LinkedList<GcLink, PyObject>>; 3],
    /// Frozen/permanent objects (excluded from normal GC)
    permanent_list: PyRwLock<LinkedList<GcLink, PyObject>>,
    /// Number of tracked objects per generation, across all interpreters.
    ///
    /// Advisory: they drive the collection threshold and `gc.get_count()`, and
    /// the generation locks — not these counters — order the list changes they
    /// describe. Every access is therefore relaxed, which keeps the tracking and
    /// untracking of every object off the barrier path.
    counts: [AtomicUsize; 3],
    /// Number of frozen objects. Advisory, like `counts`.
    permanent_count: AtomicUsize,
    /// Mutex for collection (prevents concurrent collections)
    collecting: PyMutex<()>,
    /// Next `gc_owner` tag to hand to an interpreter.
    next_owner: AtomicU16,
    /// Tags of interpreters that are gone. Their objects outlived them, so a
    /// collection adopts them — tags them `GC_NO_OWNER` again — as it walks,
    /// rather than leaving them for a collector that will never come.
    retired: PyMutex<Vec<GcOwner>>,
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
    pub const fn new() -> Self {
        Self {
            generation_lists: [
                PyRwLock::new(LinkedList::new()),
                PyRwLock::new(LinkedList::new()),
                PyRwLock::new(LinkedList::new()),
            ],
            permanent_list: PyRwLock::new(LinkedList::new()),
            counts: [
                AtomicUsize::new(0),
                AtomicUsize::new(0),
                AtomicUsize::new(0),
            ],
            permanent_count: AtomicUsize::new(0),
            collecting: PyMutex::new(()),
            next_owner: AtomicU16::new(GC_NO_OWNER + 1),
            retired: PyMutex::new(Vec::new()),
        }
    }

    /// Reserve a tag for a new interpreter. Tags are never reused; exhausting
    /// the tag space falls back to `GC_NO_OWNER`, which costs isolation but
    /// stays correct, rather than aliasing a live interpreter.
    fn alloc_owner(&self) -> GcOwner {
        self.next_owner
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
                next.checked_add(1)
            })
            .unwrap_or(GC_NO_OWNER)
    }

    /// Record that `owner`'s interpreter is gone, so the next collection adopts
    /// whatever it left behind. Retagging the objects here would mean walking
    /// every list under an interpreter drop, which happens while a collection
    /// holds the collecting lock.
    fn retire_owner(&self, owner: GcOwner) {
        if owner == GC_NO_OWNER {
            return;
        }
        self.retired.lock().push(owner);
    }

    /// Get counts for all generations. Tracked objects are shared, so these are
    /// process-wide even though the thresholds they are compared against are
    /// per interpreter.
    pub fn get_count(&self) -> (usize, usize, usize) {
        (
            self.counts[0].load(Ordering::Relaxed),
            self.counts[1].load(Ordering::Relaxed),
            self.counts[2].load(Ordering::Relaxed),
        )
    }

    /// Track a new object (add to gen0) as owned by `owner`.
    /// O(1) — intrusive linked list push_front, no hashing.
    ///
    /// # Safety
    /// obj must be a valid pointer to a PyObject
    pub unsafe fn track_object(&self, obj: NonNull<PyObject>, owner: GcOwner) {
        let obj_ref = unsafe { obj.as_ref() };
        obj_ref.set_gc_tracked();
        obj_ref.set_gc_generation(0);
        obj_ref.set_gc_owner(owner);

        self.generation_lists[0].write().push_front(obj);
        self.counts[0].fetch_add(1, Ordering::Relaxed);
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
                    &self.counts[obj_gen as usize],
                )
            } else if obj_gen == GC_PERMANENT {
                (&self.permanent_list, &self.permanent_count)
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
                release_count(count);
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

    /// Get the objects `owner` tracks (for gc.get_objects), plus the ones no
    /// interpreter owns.
    /// If generation is None, returns all such objects.
    /// If generation is Some(n), returns those in generation n only.
    pub fn get_objects(&self, generation: Option<i32>, owner: GcOwner) -> Vec<PyObjectRef> {
        fn collect_from_list(
            list: &LinkedList<GcLink, PyObject>,
            owner: GcOwner,
        ) -> impl Iterator<Item = PyObjectRef> + '_ {
            list.iter()
                .filter(move |obj| is_owned_by(obj, owner))
                .filter_map(|obj| obj.try_to_owned())
        }

        match generation {
            None => {
                // Return all tracked objects from all generations + permanent
                let mut result = Vec::new();
                for gen_list in &self.generation_lists {
                    result.extend(collect_from_list(&gen_list.read(), owner));
                }
                result.extend(collect_from_list(&self.permanent_list.read(), owner));
                result
            }
            Some(g) if (0..=2).contains(&g) => {
                let guard = self.generation_lists[g as usize].read();
                collect_from_list(&guard, owner).collect()
            }
            _ => Vec::new(),
        }
    }

    /// Check if automatic GC should run and run it if needed.
    /// Called after object allocation.
    /// Returns true if GC was run, false otherwise.
    fn maybe_collect(&self, gc: &GcInterpreterState) -> bool {
        if !gc.is_enabled() {
            return false;
        }

        // Check gen0 threshold
        let count0 = self.counts[0].load(Ordering::Relaxed) as u32;
        let threshold0 = gc.generations[0].threshold();
        if threshold0 > 0 && count0 >= threshold0 {
            #[cfg(feature = "threading")]
            {
                // Defer to the next bytecode safepoint. Collecting here would
                // stop the world while this thread may hold an internal lock
                // (e.g. a lazily-initialized frame locals cell) that another
                // thread is blocked on with no way to reach a safepoint —
                // a deadlock. At a safepoint no such lock is held.
                crate::signal::schedule_gc();
                return false;
            }
            // Without threading there is no safepoint to defer to and no other
            // thread whose frames could be read mid-mutation, so collect inline.
            #[cfg(not(feature = "threading"))]
            {
                self.collect_inner(gc, 0, false);
                return true;
            }
        }

        false
    }

    fn collect_inner(
        &self,
        gc: &GcInterpreterState,
        generation: usize,
        force: bool,
    ) -> CollectResult {
        if !force && !gc.is_enabled() {
            return CollectResult::default();
        }

        // Try to acquire the collecting lock
        let Some(_guard) = self.collecting.try_lock() else {
            return CollectResult::default();
        };

        let start_time = cfg_select! {
            target_arch = "wasm32" => (),
            _ => std::time::Instant::now(),
        };

        // Memory barrier to ensure visibility of all reference count updates
        // from other threads before we start analyzing the object graph.
        core::sync::atomic::fence(Ordering::SeqCst);

        let generation = generation.min(2);
        let debug = gc.get_debug();

        // Clear the method cache to release strong references that
        // might prevent cycle collection (_PyType_ClearCache).
        crate::builtins::type_::type_cache_clear();

        // Backstop for QSBR reclamation (threads may have missed requests).
        #[cfg(feature = "threading")]
        crate::object::qsbr::QSBR.process();

        // Stop the world before reading any tracked object's interpreter
        // state. Requested *before* the generation read locks are taken: a
        // thread parking at a safepoint may still hold a generation write lock
        // (track/untrack/promote) and must be able to release it to reach the
        // safepoint. It could not do so if this thread already held a read
        // lock it was waiting behind — hence the ordering.
        //
        // Auto-collection is deferred to a bytecode safepoint (see
        // `maybe_collect`), where no internal lock is held, so it never stops
        // the world under a lock. Explicit `gc.collect()` runs synchronously
        // here; a re-entrant call from a finalizer during an in-progress
        // collection is turned into a no-op by the `collecting` try_lock above.
        // The one residual is an explicit `gc.collect()` reached from a
        // finalizer/`__del__` that runs inline while a non-generation internal
        // lock is still held (e.g. a container write lock during element
        // replacement) with another thread blocked on that same lock: stopping
        // the world then waits for a thread that cannot reach a safepoint.
        // Closing it fully would require making those locks stop-the-world
        // aware; the exclusion above only serializes the fork/GC requesters.
        #[cfg(feature = "threading")]
        let mut stw = CollectStopTheWorld::new();

        // Step 1: Gather objects from generations 0..=generation
        // Hold read locks for the entire scan to prevent concurrent modifications.
        let gen_locks: Vec<_> = (0..=generation)
            .map(|i| self.generation_lists[i].read())
            .collect();

        // Only this interpreter's objects, plus the ones no interpreter owns.
        // Another interpreter's objects stay out of the candidate set, so they
        // act as external roots: anything they reference survives this pass.
        let owner = gc.owner;
        // Sorted so that the test below, which every scanned object pays for,
        // stays logarithmic in the number of interpreters that have been
        // dropped instead of linear.
        let retired = {
            let mut retired = self.retired.lock().clone();
            retired.sort_unstable();
            retired
        };
        let mut collecting: GcSet<GcPtr> = GcSet::default();
        for gen_list in &gen_locks {
            for obj in gen_list.iter() {
                if retired.binary_search(&obj.gc_owner()).is_ok() {
                    obj.set_gc_owner(GC_NO_OWNER);
                }
                if obj.strong_count() > 0 && is_owned_by(obj, owner) {
                    collecting.insert(GcPtr(NonNull::from(obj)));
                }
            }
        }

        // A full collection is the only one that sees every generation, so it
        // is where adoption finishes and the tags stop being tracked.
        if generation == 2 && !retired.is_empty() {
            for obj in self.permanent_list.read().iter() {
                if retired.binary_search(&obj.gc_owner()).is_ok() {
                    obj.set_gc_owner(GC_NO_OWNER);
                }
            }
            // Only the tags this scan saw: one retired while it ran still has
            // objects nobody has adopted.
            self.retired
                .lock()
                .retain(|tag| retired.binary_search(tag).is_err());
        }

        if collecting.is_empty() {
            // Reset counts for generations whose objects were promoted away.
            // For gen2 (oldest), survivors stay in-place so don't reset gen2 count.
            let reset_end = if generation >= 2 { 2 } else { generation + 1 };
            for i in 0..reset_end {
                self.counts[i].store(0, Ordering::Relaxed);
            }

            let duration = elapsed_secs(start_time);

            gc.generations[generation].update_stats(0, 0, 0, duration);
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
        let mut gc_refs: GcMap<GcPtr, usize> = GcMap::default();

        #[expect(
            clippy::iter_over_hash_type,
            reason = "Iteration order doesn't matter here"
        )]
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
        let mut referents_map: GcMap<GcPtr, Vec<NonNull<PyObject>>> = GcMap::default();

        #[expect(
            clippy::iter_over_hash_type,
            reason = "Iteration order doesn't matter here"
        )]
        for &ptr in &collecting {
            let obj = unsafe { ptr.0.as_ref() };
            if obj.strong_count() == 0 {
                continue;
            }
            let referent_ptrs = unsafe { obj.gc_get_referent_ptrs() };
            for &child_ptr in &referent_ptrs {
                let gc_ptr = GcPtr(child_ptr);
                if collecting.contains(&gc_ptr)
                    && let Some(refs) = gc_refs.get_mut(&gc_ptr)
                {
                    *refs = refs.saturating_sub(1);
                }
            }
            referents_map.insert(ptr, referent_ptrs);
        }

        // Step 4: Find reachable objects (gc_refs > 0) and traverse from them
        let mut reachable: GcSet<GcPtr> = GcSet::default();
        let mut worklist: Vec<GcPtr> = Vec::new();

        #[expect(
            clippy::iter_over_hash_type,
            reason = "Iteration order doesn't matter here"
        )]
        for (&ptr, &refs) in &gc_refs {
            if refs > 0 {
                reachable.insert(ptr);
                worklist.push(ptr);
            }
        }

        while let Some(ptr) = worklist.pop() {
            let obj = unsafe { ptr.0.as_ref() };
            if obj.is_gc_tracked() {
                // Reuse the pre-computed referent pointers from step 3, in
                // place: copying them out again costs a second pass over every
                // edge in the heap. Objects skipped in step 3 (strong_count was
                // 0) have none stored and are traversed here instead.
                let computed;
                let referent_ptrs: &[NonNull<PyObject>] = match referents_map.get(&ptr) {
                    Some(stored) => stored,
                    None => {
                        computed = unsafe { obj.gc_get_referent_ptrs() };
                        &computed
                    }
                };
                for &child_ptr in referent_ptrs {
                    let gc_ptr = GcPtr(child_ptr);
                    if collecting.contains(&gc_ptr) && reachable.insert(gc_ptr) {
                        worklist.push(gc_ptr);
                    }
                }
            }
        }

        // Step 5: Find unreachable objects
        let unreachable: Vec<GcPtr> = collecting.difference(&reachable).copied().collect();

        // With the world stopped, every frame on any thread's call stack is a
        // live root that is externally referenced and must have been
        // classified reachable. A running frame appearing in `unreachable`
        // would mean the reachability analysis observed its interpreter state
        // as garbage — the exact hazard the barrier exists to prevent.
        // Verify no running frame is classified unreachable.
        // Walk the TLS frame chain (CURRENT_FRAME) instead of top_frame,
        // because stack-allocated frames update only CURRENT_FRAME (via
        // set_current_frame_nosave), not top_frame.
        #[cfg(all(unix, feature = "threading", debug_assertions))]
        if stw.is_stopped() {
            let unreachable_set: GcSet<GcPtr> = unreachable.iter().copied().collect();
            let mut cur = crate::vm::thread::get_current_frame();
            while !cur.is_null() {
                let iframe = unsafe { &*cur };
                if let Some(fo) = iframe.frame_obj() {
                    let obj = fo.as_object();
                    let ptr = GcPtr(NonNull::from(obj));
                    debug_assert!(
                        !unreachable_set.contains(&ptr),
                        "running frame {obj:p} classified unreachable during GC"
                    );
                }
                cur = iframe.previous();
            }
        }

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

        // The pointer-reading phases are done: strong references now pin every
        // survivor and unreachable object, so the remaining phases can run with
        // the world restarted. Finalizers and tp_clear must not run under
        // stop-the-world — they execute arbitrary Python — and they only touch
        // dead/husk objects, never a running frame.
        #[cfg(feature = "threading")]
        stw.restart();

        if unreachable.is_empty() {
            drop(gen_locks);
            self.promote_survivors(generation, &survivor_refs);
            let reset_end = if generation >= 2 { 2 } else { generation + 1 };
            for i in 0..reset_end {
                self.counts[i].store(0, Ordering::Relaxed);
            }

            let duration = elapsed_secs(start_time);

            gc.generations[generation].update_stats(0, 0, candidates, duration);
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
                self.counts[i].store(0, Ordering::Relaxed);
            }

            let duration = elapsed_secs(start_time);

            gc.generations[generation].update_stats(0, 0, candidates, duration);
            return CollectResult {
                collected: 0,
                uncollectable: 0,
                candidates,
                duration,
            };
        }

        // 6b: Record initial strong counts (for resurrection detection)
        let initial_counts: GcMap<GcPtr, usize> = unreachable_refs
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
        let mut resurrected_set: GcSet<GcPtr> = GcSet::default();
        let unreachable_set: GcSet<GcPtr> = unreachable.iter().copied().collect();

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
            let dead_ptrs: GcSet<usize> = truly_dead
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
            let mut garbage_guard = gc.garbage.lock();
            for obj_ref in &truly_dead {
                garbage_guard.push(obj_ref.clone());
            }
        }

        if !truly_dead.is_empty() {
            // Break cycles by clearing references (tp_clear)
            // Use deferred drop context to prevent stack overflow.
            // With DEBUG_SAVEALL the objects stay reachable through
            // gc.garbage, so they must not be cleared (delete_garbage
            // skips tp_clear for saved objects).
            let save_all = debug.contains(GcDebugFlags::SAVEALL);

            // Untrack dead objects BEFORE clearing them, mirroring the
            // untrack-then-clear ordering of the refcount dealloc path.
            // A cleared object (e.g. a frame husk with iframe == None) must
            // never be observable through the generation lists, or another
            // thread could obtain a strong reference via gc.get_objects()
            // and access the cleared payload.
            let mut late_resurrected: GcSet<GcPtr> = GcSet::default();
            if !save_all {
                let mut expected_counts: GcMap<GcPtr, usize> = GcMap::default();
                for obj_ref in &truly_dead {
                    let obj = obj_ref.as_ref();
                    if obj.is_gc_tracked() {
                        unsafe { self.untrack_object(NonNull::from(obj)) };
                    }
                    // One strong reference held by the `truly_dead` vec itself.
                    expected_counts.insert(GcPtr(NonNull::from(obj)), 1);
                }
                // With the objects out of the generation lists, no new external
                // reference can appear. Count the references coming from within
                // the dead set; any surplus in strong_count means another thread
                // grabbed a reference before untracking (late resurrection) and
                // the object must not be cleared.
                let mut referents: GcMap<GcPtr, Vec<NonNull<PyObject>>> = GcMap::default();
                for obj_ref in &truly_dead {
                    let referent_ptrs = unsafe { obj_ref.gc_get_referent_ptrs() };
                    for child_ptr in &referent_ptrs {
                        if let Some(n) = expected_counts.get_mut(&GcPtr(*child_ptr)) {
                            *n += 1;
                        }
                    }
                    referents.insert(GcPtr(NonNull::from(obj_ref.as_ref())), referent_ptrs);
                }
                let mut worklist: Vec<GcPtr> = Vec::new();
                for obj_ref in &truly_dead {
                    let ptr = GcPtr(NonNull::from(obj_ref.as_ref()));
                    if obj_ref.strong_count() > expected_counts[&ptr]
                        && late_resurrected.insert(ptr)
                    {
                        worklist.push(ptr);
                    }
                }
                // A holder of a late-resurrected object can reach its referents,
                // so everything reachable from it must stay intact as well.
                while let Some(ptr) = worklist.pop() {
                    let Some(referent_ptrs) = referents.get(&ptr) else {
                        continue;
                    };
                    for child_ptr in referent_ptrs {
                        let child = GcPtr(*child_ptr);
                        if expected_counts.contains_key(&child) && late_resurrected.insert(child) {
                            worklist.push(child);
                        }
                    }
                }
                // Re-track late-resurrected objects so a future collection can
                // retry once the external references are released.
                #[expect(
                    clippy::iter_over_hash_type,
                    reason = "Iteration order doesn't matter here"
                )]
                for &ptr in &late_resurrected {
                    // Re-tracking a resurrected object: it keeps the owner it
                    // was allocated under.
                    let owner = unsafe { ptr.0.as_ref() }.gc_owner();
                    unsafe { self.track_object(ptr.0, owner) };
                }
            }
            rustpython_common::refcount::with_deferred_drops(|| {
                if !save_all {
                    for obj_ref in &truly_dead {
                        let obj = obj_ref.as_ref();
                        if late_resurrected.contains(&GcPtr(NonNull::from(obj))) {
                            continue;
                        }
                        if obj.gc_has_clear() {
                            let edges = unsafe { obj.gc_clear() };
                            drop(edges);
                        }
                    }
                }
                drop(truly_dead);
            });
        }

        // Reset counts for generations whose objects were promoted away.
        // For gen2 (oldest), survivors stay in-place so don't reset gen2 count.
        let reset_end = if generation >= 2 { 2 } else { generation + 1 };
        for i in 0..reset_end {
            self.counts[i].store(0, Ordering::Relaxed);
        }

        let duration = elapsed_secs(start_time);

        gc.generations[generation].update_stats(collected, 0, candidates, duration);

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
                    release_count(&self.counts[src_gen]);

                    dst.push_front(ptr);
                    self.counts[next_gen].fetch_add(1, Ordering::Relaxed);

                    obj.set_gc_generation(next_gen as u8);
                }
            }
        }
    }

    /// Get count of frozen objects
    pub fn get_freeze_count(&self) -> usize {
        self.permanent_count.load(Ordering::Relaxed)
    }

    /// Freeze the objects `owner` could collect (move them to the permanent
    /// generation).
    /// Lock order: generation_lists[i] → permanent_list (consistent with unfreeze).
    fn freeze(&self, owner: GcOwner) {
        let mut count = 0usize;

        for (gen_idx, gen_list) in self.generation_lists.iter().enumerate() {
            let mut list = gen_list.write();
            let mut perm = self.permanent_list.write();
            let moving: Vec<_> = list
                .iter()
                .filter(|obj| is_owned_by(obj, owner))
                .map(NonNull::from)
                .collect();
            for ptr in moving {
                if unsafe { list.remove(ptr) }.is_none() {
                    continue;
                }
                perm.push_front(ptr);
                unsafe { ptr.as_ref().set_gc_generation(GC_PERMANENT) };
                count += 1;
                release_count(&self.counts[gen_idx]);
            }
        }

        self.permanent_count.fetch_add(count, Ordering::Relaxed);
    }

    /// Unfreeze the objects `owner` froze (move them from permanent to gen2).
    /// Lock order: generation_lists[2] → permanent_list (consistent with freeze).
    fn unfreeze(&self, owner: GcOwner) {
        let mut count = 0usize;

        {
            let mut gen2 = self.generation_lists[2].write();
            let mut perm_list = self.permanent_list.write();
            let moving: Vec<_> = perm_list
                .iter()
                .filter(|obj| is_owned_by(obj, owner))
                .map(NonNull::from)
                .collect();
            for ptr in moving {
                if unsafe { perm_list.remove(ptr) }.is_none() {
                    continue;
                }
                gen2.push_front(ptr);
                unsafe { ptr.as_ref().set_gc_generation(2) };
                count += 1;
            }
            let _ = self.permanent_count.fetch_update(
                Ordering::Relaxed,
                Ordering::Relaxed,
                |permanent| Some(permanent.saturating_sub(count)),
            );
        }

        self.counts[2].fetch_add(count, Ordering::Relaxed);
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
            reinit_mutex_after_fork(&self.retired);

            for rw in &self.generation_lists {
                reinit_rwlock_after_fork(rw);
            }
            reinit_rwlock_after_fork(&self.permanent_list);
        }
    }
}

/// Per-interpreter garbage collector state (≈ `PyInterpreterState.gc`).
///
/// The generation lists are process-wide (see [`GcState`]); what an interpreter
/// owns is the policy applied to them and the results — which objects its
/// collections consider, whether they run automatically, and where uncollectable
/// objects end up.
pub struct GcInterpreterState {
    /// Tag written into every object this interpreter tracks.
    owner: GcOwner,
    /// Per-generation thresholds and statistics.
    pub generations: [GcGeneration; 3],
    /// GC enabled flag
    enabled: AtomicBool,
    /// Debug flags
    debug: AtomicU32,
    /// Uncollectable objects saved by this interpreter's collections, drained
    /// into `py_garbage` by `gc.collect()`.
    pub garbage: PyMutex<Vec<PyObjectRef>>,
    /// `gc.garbage`
    pub py_garbage: crate::builtins::PyListRef,
    /// `gc.callbacks`
    pub py_callbacks: crate::builtins::PyListRef,
}

impl GcInterpreterState {
    pub fn new(ctx: &crate::vm::Context) -> Self {
        Self {
            owner: gc_state().alloc_owner(),
            generations: [
                GcGeneration::new(2000), // young
                GcGeneration::new(10),   // old[0]
                GcGeneration::new(0),    // old[1]
            ],
            enabled: AtomicBool::new(true),
            debug: AtomicU32::new(0),
            garbage: PyMutex::new(Vec::new()),
            py_garbage: ctx.new_list(Vec::new()),
            py_callbacks: ctx.new_list(Vec::new()),
        }
    }

    /// Check if GC is enabled.
    ///
    /// Relaxed, like [`GcGeneration::threshold`]: it is read once per
    /// allocation, and an allocation racing `gc.disable()` may use either value.
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// Enable GC
    pub fn enable(&self) {
        self.enabled.store(true, Ordering::Relaxed);
    }

    /// Disable GC
    pub fn disable(&self) {
        self.enabled.store(false, Ordering::Relaxed);
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

    /// Get statistics for all generations
    pub fn get_stats(&self) -> [GcStats; 3] {
        [
            self.generations[0].stats(),
            self.generations[1].stats(),
            self.generations[2].stats(),
        ]
    }

    /// Perform garbage collection on the given generation
    pub fn collect(&self, generation: usize) -> CollectResult {
        gc_state().collect_inner(self, generation, false)
    }

    /// Force collection even if GC is disabled (for manual gc.collect() calls)
    pub fn collect_force(&self, generation: usize) -> CollectResult {
        gc_state().collect_inner(self, generation, true)
    }

    /// The tracked objects this interpreter can reach (for gc.get_objects).
    pub fn get_objects(&self, generation: Option<i32>) -> Vec<PyObjectRef> {
        gc_state().get_objects(generation, self.owner)
    }

    /// Move the objects this interpreter could collect into the permanent
    /// generation.
    pub fn freeze(&self) {
        gc_state().freeze(self.owner);
    }

    /// Move them back out of it.
    pub fn unfreeze(&self) {
        gc_state().unfreeze(self.owner);
    }

    /// Reset this interpreter's GC locks to unlocked state after fork().
    ///
    /// # Safety
    /// Must only be called after fork() in the child process when no other
    /// threads exist. The calling thread must NOT hold any of these locks.
    #[cfg(all(unix, feature = "threading"))]
    pub unsafe fn reinit_after_fork(&self) {
        unsafe {
            crate::common::lock::reinit_mutex_after_fork(&self.garbage);
            for generation in &self.generations {
                generation.reinit_stats_after_fork();
            }
        }
    }
}

impl Drop for GcInterpreterState {
    fn drop(&mut self) {
        // Objects this interpreter tracked can outlive it (another interpreter
        // may still hold one). Clearing the tag hands them to every collection
        // instead of stranding them. The tag itself is not handed back: it stays
        // retired so that a later interpreter cannot inherit these objects.
        gc_state().retire_owner(self.owner);
    }
}

/// The tag `track_object` should write for the interpreter running now.
#[must_use]
pub fn current_owner() -> GcOwner {
    // SAFETY: the pointee is owned by the `PyGlobalState` of the VM on top of
    // this thread's VM stack, which outlives the section this call runs in.
    crate::vm::thread::current_gc_state().map_or(GC_NO_OWNER, |gc| unsafe { gc.as_ref() }.owner)
}

/// Track a freshly allocated object under the interpreter running now, and let
/// it collect if the allocation pushed gen0 past its threshold.
///
/// # Safety
/// obj must be a valid pointer to a PyObject that is not already tracked.
pub(crate) unsafe fn track_new_object(obj: NonNull<PyObject>) {
    let state = gc_state();
    let Some(gc) = crate::vm::thread::current_gc_state() else {
        // No interpreter is running: the shared context builds its own objects
        // this way. They are left unowned, so every interpreter collects them.
        unsafe { state.track_object(obj, GC_NO_OWNER) };
        return;
    };
    // SAFETY: as in `current_owner`.
    let gc = unsafe { gc.as_ref() };
    unsafe { state.track_object(obj, gc.owner) };
    state.maybe_collect(gc);
}

/// Get a reference to the GC state.
///
/// In threading mode this is a true global (OnceLock).
/// In non-threading mode this is thread-local, because PyRwLock/PyMutex
/// use Cell-based locks that are not Sync.
///
/// Every interpreter's tracked objects live in these lists, because untracking
/// happens in `default_dealloc`, where no interpreter is in scope to route to.
/// What a collection *acts on* is still one interpreter's own objects, selected
/// by the `gc_owner` tag; [`GcInterpreterState`] holds the rest of the state
/// that goes with that. The counts here, and so `gc.get_count()` and
/// `gc.get_freeze_count()`, stay process-wide: they measure how full these
/// lists are.
pub fn gc_state() -> &'static GcState {
    rustpython_common::static_cell! {
        static GC_STATE: GcState;
    }
    GC_STATE.get_or_init(GcState::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interpreter_state() -> GcInterpreterState {
        GcInterpreterState::new(crate::vm::Context::genesis())
    }

    #[test]
    fn gc_state_default() {
        let state = interpreter_state();
        assert!(state.is_enabled());
        assert_eq!(state.get_debug(), GcDebugFlags::empty());
        assert_eq!(state.get_threshold(), (2000, 10, 0));
    }

    #[test]
    fn gc_enable_disable() {
        let state = interpreter_state();
        assert!(state.is_enabled());
        state.disable();
        assert!(!state.is_enabled());
        state.enable();
        assert!(state.is_enabled());
    }

    #[test]
    fn gc_threshold() {
        let state = interpreter_state();
        state.set_threshold(100, Some(20), Some(30));
        assert_eq!(state.get_threshold(), (100, 20, 30));
    }

    #[test]
    fn gc_debug_flags() {
        let state = interpreter_state();
        state.set_debug(GcDebugFlags::STATS | GcDebugFlags::COLLECTABLE);
        assert_eq!(
            state.get_debug(),
            GcDebugFlags::STATS | GcDebugFlags::COLLECTABLE
        );
    }

    /// Live interpreters never share an owner tag, or their collections would
    /// reach each other's objects.
    #[test]
    fn gc_owner_tags_are_distinct_while_live() {
        let first = interpreter_state();
        let second = interpreter_state();
        assert_ne!(first.owner, second.owner);
        assert_ne!(first.owner, GC_NO_OWNER);
        assert_ne!(second.owner, GC_NO_OWNER);
    }
}
