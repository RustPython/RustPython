//! Garbage Collection State and Algorithm
//!
//! Interpreter-local heaps and a compact, revalidated cycle-collection snapshot.

use crate::common::{lock::PyMutex, rc::PyRc};
use crate::object::{GC_PERMANENT, GC_UNTRACKED};
use crate::{AsObject, PyObject, PyObjectRef};
use core::ptr::NonNull;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

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
/// Object membership and allocation pressure belong to this interpreter's heap.
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

mod collector;
mod graph;
mod heap;

use graph::Graph;
pub(crate) use heap::GcHandle;
pub use heap::GcHeap;

type HeapWeak = cfg_select! {
    feature = "threading" => alloc::sync::Weak::<GcHeap>,
    _ => alloc::rc::Weak::<GcHeap>,
};

/// Collection frequency is independent of heap occupancy.
#[derive(Default)]
struct GcSchedule {
    young_collections: usize,
    middle_collections: usize,
    long_lived_pending: usize,
    long_lived_total: usize,
}

impl GcSchedule {
    fn generation(&self, thresholds: (u32, u32, u32)) -> usize {
        // As in CPython, require growth before scanning the entire old heap.
        // A full collection every fixed number of allocations would make
        // building a large, retained heap quadratic.
        if self.middle_collections > thresholds.2 as usize
            && self.long_lived_pending >= self.long_lived_total / 4
        {
            2
        } else {
            usize::from(self.young_collections > thresholds.1 as usize)
        }
    }

    fn start_collection(&mut self, generation: usize) {
        if generation == 0 {
            self.young_collections = self.young_collections.saturating_add(1);
        } else {
            self.young_collections = 0;
            self.middle_collections = if generation == 1 {
                self.middle_collections.saturating_add(1)
            } else {
                0
            };
        }
    }

    fn record_survivors(&mut self, generation: usize, survivors: usize) {
        if generation == 1 {
            self.long_lived_pending = self.long_lived_pending.saturating_add(survivors);
        } else if generation == 2 {
            self.long_lived_pending = 0;
            self.long_lived_total = survivors;
        }
    }
}

/// Parks only the candidate heap's owner while reading Python pointers.
/// Graph analysis and finalizers run after its threads resume.
#[cfg(feature = "threading")]
struct CollectStopTheWorld {
    state: Option<PyRc<crate::vm::PyGlobalState>>,
    restarted: bool,
}

#[cfg(feature = "threading")]
impl CollectStopTheWorld {
    fn new(gc: &GcInterpreterState) -> Self {
        let state = gc
            .heap
            .owner
            .and_then(crate::vm::runtime::lookup_interpreter);
        if let Some(state) = &state {
            debug_assert!(core::ptr::eq(gc, &state.gc));
            state.stop_the_world.stop_the_world(state);
        }
        // A standalone, unpublished native heap has no executing interpreter.
        Self {
            state,
            restarted: false,
        }
    }

    fn restart(&mut self) {
        if !self.restarted {
            self.restarted = true;
            if let Some(state) = &self.state {
                state.stop_the_world.start_the_world(state);
            }
        }
    }
}

impl Drop for CollectStopTheWorld {
    fn drop(&mut self) {
        self.restart();
    }
}

#[cfg(not(feature = "threading"))]
struct CollectStopTheWorld;
#[cfg(not(feature = "threading"))]
impl CollectStopTheWorld {
    fn new(_gc: &GcInterpreterState) -> Self {
        Self
    }
    fn restart(&mut self) {}
}

/// Process coordination and weak heap directory. Object operations go directly
/// through the owning heap, including deallocation outside an entered VM.
pub struct GcState {
    shared: PyRc<GcHeap>,
    heaps: PyMutex<Vec<HeapWeak>>,
    collecting: PyMutex<()>,
}

impl Default for GcState {
    fn default() -> Self {
        Self::new()
    }
}

impl GcState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            shared: PyRc::new(GcHeap::new(None)),
            heaps: PyMutex::new(Vec::new()),
            collecting: PyMutex::new(()),
        }
    }

    fn new_heap(&self, owner: i64) -> PyRc<GcHeap> {
        let heap = PyRc::new(GcHeap::new(Some(owner)));
        let mut heaps = self.heaps.lock();
        heaps.retain(|h| h.strong_count() != 0);
        heaps.push(PyRc::downgrade(&heap));
        heap
    }

    fn retired_heaps(&self) -> Vec<PyRc<GcHeap>> {
        let mut result = Vec::new();
        self.heaps.lock().retain(|weak| {
            if let Some(heap) = weak.upgrade() {
                if heap.retired.load(Ordering::Acquire) {
                    result.push(heap);
                }
                true
            } else {
                false
            }
        });
        result
    }

    /// Track a valid untracked object. A previously bound object retains its
    /// original heap even if it is re-tracked from a different interpreter.
    ///
    /// # Safety
    /// The caller must exclude concurrent tracking of the object.
    pub unsafe fn track_object(&self, obj: NonNull<PyObject>, heap: PyRc<GcHeap>) {
        unsafe { GcHeap::track(&heap, obj, false) };
    }

    /// # Safety
    /// The object's memory must remain valid throughout removal.
    pub unsafe fn untrack_object(&self, obj: NonNull<PyObject>) {
        let obj = unsafe { obj.as_ref() };
        if let Some(heap) = obj.gc_handle().heap() {
            heap.untrack(obj);
        }
    }

    fn maybe_collect(&self, gc: &GcInterpreterState) -> bool {
        let threshold = gc.generations[0].threshold();
        if gc.is_enabled()
            && threshold != 0
            && gc.heap.allocations.load(Ordering::Relaxed) as i128 >= threshold as i128
        {
            #[cfg(feature = "threading")]
            gc.scheduled.store(true, Ordering::Relaxed);
            #[cfg(not(feature = "threading"))]
            {
                self.collect_inner(gc, None, false);
                return true;
            }
        }
        false
    }

    #[cfg(all(unix, feature = "threading"))]
    pub(crate) fn lock_retired_for_fork(&self) -> crate::common::lock::PyMutexGuard<'_, ()> {
        self.collecting.lock()
    }

    #[cfg(all(unix, feature = "threading"))]
    pub(crate) fn heaps_for_fork(&self) -> Vec<PyRc<GcHeap>> {
        core::iter::once(self.shared.clone())
            .chain(self.heaps.lock().iter().filter_map(HeapWeak::upgrade))
            .collect()
    }

    #[cfg(all(unix, feature = "threading"))]
    /// # Safety
    /// Call only in the fork child after the fork coordinator stabilized all
    /// heaps and released its inherited guards.
    pub unsafe fn reinit_after_fork(&self) {
        unsafe {
            crate::common::lock::reinit_mutex_after_fork(&self.collecting);
            crate::common::lock::reinit_mutex_after_fork(&self.heaps);
            self.shared.reinit_after_fork();
            for heap in self.heaps.lock().iter().filter_map(HeapWeak::upgrade) {
                heap.reinit_after_fork();
            }
        }
    }
}

/// Policy, allocation pressure and collection results for one interpreter.
pub struct GcInterpreterState {
    heap: PyRc<GcHeap>,
    collecting: PyMutex<()>,
    #[cfg(all(unix, feature = "threading"))]
    collecting_thread: core::sync::atomic::AtomicU64,
    pub generations: [GcGeneration; 3],
    schedule: PyMutex<GcSchedule>,
    enabled: AtomicBool,
    #[cfg(feature = "threading")]
    scheduled: AtomicBool,
    debug: AtomicU32,
    pub garbage: PyMutex<Vec<PyObjectRef>>,
    py_garbage: PyMutex<Option<crate::builtins::PyListRef>>,
    py_callbacks: PyMutex<Option<crate::builtins::PyListRef>>,
}

impl GcInterpreterState {
    #[cfg(feature = "threading")]
    pub(crate) fn qsbr(&self) -> &alloc::sync::Arc<crate::object::qsbr::Qsbr> {
        &self.heap.qsbr
    }

    pub fn new(ctx: &crate::vm::Context) -> Self {
        Self::new_for_interpreter(ctx, crate::vm::runtime::alloc_interpreter_id())
    }

    pub(crate) fn new_for_interpreter(ctx: &crate::vm::Context, owner: i64) -> Self {
        let heap = gc_state().new_heap(owner);
        let _owner = AllocationScope::new(heap.clone());
        let state = Self {
            heap,
            collecting: PyMutex::new(()),
            #[cfg(all(unix, feature = "threading"))]
            collecting_thread: core::sync::atomic::AtomicU64::new(0),
            generations: [
                GcGeneration::new(2000),
                GcGeneration::new(10),
                GcGeneration::new(10),
            ],
            schedule: PyMutex::new(GcSchedule::default()),
            enabled: AtomicBool::new(true),
            #[cfg(feature = "threading")]
            scheduled: AtomicBool::new(false),
            debug: AtomicU32::new(0),
            garbage: PyMutex::new(Vec::new()),
            py_garbage: PyMutex::new(Some(ctx.new_list(Vec::new()))),
            py_callbacks: PyMutex::new(Some(ctx.new_list(Vec::new()))),
        };
        state.heap.allocations.store(0, Ordering::Relaxed);
        state
    }

    pub(crate) fn allocation_scope(&self) -> AllocationScope {
        AllocationScope::new(self.heap.clone())
    }

    pub(crate) fn garbage_list(&self) -> Option<crate::builtins::PyListRef> {
        self.py_garbage.lock().clone()
    }

    pub(crate) fn callbacks_list(&self) -> Option<crate::builtins::PyListRef> {
        self.py_callbacks.lock().clone()
    }

    pub(crate) fn close_python_roots(&self) {
        self.disable();
        self.set_debug(GcDebugFlags::empty());
        let garbage = self.py_garbage.lock().take();
        let callbacks = self.py_callbacks.lock().take();
        let retained = core::mem::take(&mut *self.garbage.lock());
        drop((garbage, callbacks, retained));
    }

    pub(crate) fn retire_python_roots(&self, ctx: &crate::Context) {
        self.disable();
        self.set_debug(GcDebugFlags::empty());
        let empty_garbage = ctx.new_list(Vec::new());
        let empty_callbacks = ctx.new_list(Vec::new());
        let garbage = self.py_garbage.lock().replace(empty_garbage);
        let callbacks = self.py_callbacks.lock().replace(empty_callbacks);
        let retained = core::mem::take(&mut *self.garbage.lock());
        drop((garbage, callbacks, retained));
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }
    pub fn enable(&self) {
        self.enabled.store(true, Ordering::Relaxed);
    }
    pub fn disable(&self) {
        self.enabled.store(false, Ordering::Relaxed);
    }
    pub fn get_debug(&self) -> GcDebugFlags {
        GcDebugFlags::from_bits_truncate(self.debug.load(Ordering::Relaxed))
    }
    pub fn set_debug(&self, flags: GcDebugFlags) {
        self.debug.store(flags.bits(), Ordering::Relaxed);
    }
    pub fn get_threshold(&self) -> (u32, u32, u32) {
        (
            self.generations[0].threshold(),
            self.generations[1].threshold(),
            self.generations[2].threshold(),
        )
    }
    pub fn set_threshold(&self, t0: u32, t1: Option<u32>, t2: Option<u32>) {
        self.generations[0].set_threshold(t0);
        if let Some(t1) = t1 {
            self.generations[1].set_threshold(t1);
        }
        if let Some(t2) = t2 {
            self.generations[2].set_threshold(t2);
        }
    }
    pub fn get_count(&self) -> (isize, usize, usize) {
        let schedule = self.schedule.lock();
        (
            self.heap.allocations.load(Ordering::Relaxed),
            schedule.young_collections,
            schedule.middle_collections,
        )
    }
    pub fn get_stats(&self) -> [GcStats; 3] {
        core::array::from_fn(|i| self.generations[i].stats())
    }
    pub fn collect(&self, generation: usize) -> CollectResult {
        gc_state().collect_inner(self, Some(generation), false)
    }
    pub fn collect_force(&self, generation: usize) -> CollectResult {
        gc_state().collect_inner(self, Some(generation), true)
    }
    #[cfg(feature = "threading")]
    pub(crate) fn collect_automatic(&self) -> CollectResult {
        gc_state().collect_inner(self, None, false)
    }
    #[cfg(feature = "threading")]
    #[inline]
    pub(crate) fn collection_ready(&self) -> bool {
        self.scheduled.load(Ordering::Relaxed) && !self.collecting.is_locked()
    }
    pub fn get_objects(&self, generation: Option<i32>) -> Vec<PyObjectRef> {
        self.heap.get_objects(generation)
    }
    pub fn freeze(&self) {
        self.heap.freeze();
        self.schedule.lock().start_collection(2);
    }
    pub fn unfreeze(&self) {
        self.heap.unfreeze();
    }
    pub fn get_freeze_count(&self) -> usize {
        self.heap.freeze_count()
    }
    #[cfg(all(unix, feature = "threading"))]
    /// # Safety
    /// Only the surviving fork thread may execute. Heap and policy mutations
    /// must have been quiescent at the syscall.
    pub unsafe fn reinit_after_fork(&self) {
        unsafe {
            // fork() can run inside a callback of this thread's collection.
            // Its native guard survives and must unlock the original mutex.
            if self.collecting_thread.load(Ordering::Relaxed) != crate::stdlib::_thread::get_ident()
            {
                crate::common::lock::reinit_mutex_after_fork(&self.collecting);
                self.collecting_thread.store(0, Ordering::Relaxed);
            }
            crate::common::lock::reinit_mutex_after_fork(&self.garbage);
            crate::common::lock::reinit_mutex_after_fork(&self.py_garbage);
            crate::common::lock::reinit_mutex_after_fork(&self.py_callbacks);
            crate::common::lock::reinit_mutex_after_fork(&self.schedule);
            for generation in &self.generations {
                generation.reinit_stats_after_fork();
            }
        }
    }
}

impl Drop for GcInterpreterState {
    fn drop(&mut self) {
        crate::vm::thread::native_sweep(|| {
            #[cfg(all(unix, feature = "threading"))]
            let _retirement = crate::vm::fork::native_phase();
            let _owner = self.allocation_scope();
            let mut cleanup = crate::vm::owned::Cleanup::default();
            cleanup.run(|| self.close_python_roots());
            self.heap.retired.store(true, Ordering::Release);
            cleanup.run(|| gc_state().collect_retired());
            cleanup.finish();
        });
    }
}

/// Heap for objects allocated on the currently entered interpreter.
#[must_use]
pub(crate) fn current_owner() -> PyRc<GcHeap> {
    if let Some(owner) = ALLOCATION_OWNER.with(|owner| owner.borrow().clone()) {
        return owner;
    }
    crate::vm::thread::current_gc_state().map_or_else(
        || gc_state().shared.clone(),
        |gc| unsafe { gc.as_ref() }.heap.clone(),
    )
}

/// The allocation domain, including bootstrap and shared-definition scopes.
pub(crate) fn current_interpreter_id() -> Option<i64> {
    ALLOCATION_OWNER.with(|owner| {
        if let Some(owner) = owner.borrow().as_ref() {
            owner.owner
        } else {
            crate::vm::thread::try_with_current_vm(|vm| vm.state.interpreter_id)
        }
    })
}

thread_local! {
    static ALLOCATION_OWNER: core::cell::RefCell<Option<PyRc<GcHeap>>> = const {
        core::cell::RefCell::new(None)
    };
}

/// Bootstrap and native-definition construction can allocate before a VM is
/// attached, or while a different interpreter is entered. Scope the owner
/// explicitly without pretending that those allocations have an active VM.
pub(crate) struct AllocationScope {
    previous: Option<PyRc<GcHeap>>,
    _thread: core::marker::PhantomData<alloc::rc::Rc<()>>,
}

impl AllocationScope {
    fn new(heap: PyRc<GcHeap>) -> Self {
        Self {
            previous: ALLOCATION_OWNER.with(|owner| owner.replace(Some(heap))),
            _thread: core::marker::PhantomData,
        }
    }

    pub(crate) fn shared() -> Self {
        Self::new(gc_state().shared.clone())
    }
}

impl Drop for AllocationScope {
    fn drop(&mut self) {
        ALLOCATION_OWNER.with(|owner| owner.replace(self.previous.take()));
    }
}

/// # Safety
/// obj must be valid, freshly initialized and not yet tracked.
pub(crate) unsafe fn track_new_object(obj: NonNull<PyObject>) {
    let state = gc_state();
    let heap = current_owner();
    unsafe { GcHeap::track(&heap, obj, true) };
    if let Some(gc) = crate::vm::thread::current_gc_state() {
        let gc = unsafe { gc.as_ref() };
        if PyRc::ptr_eq(&heap, &gc.heap) {
            state.maybe_collect(gc);
        }
    }
}

/// # Safety
/// Both objects must be valid, distinct, freshly initialized and untracked.
pub(crate) unsafe fn track_new_pair(obj: NonNull<PyObject>, frame: NonNull<PyObject>) {
    let state = gc_state();
    let gc = crate::vm::thread::current_gc_state();
    let heap = current_owner();
    unsafe { GcHeap::track_pair(&heap, obj, frame) };
    if let Some(gc) = gc {
        let gc = unsafe { gc.as_ref() };
        if PyRc::ptr_eq(&heap, &gc.heap) {
            state.maybe_collect(gc);
        }
    }
}

pub(crate) fn gc_state() -> &'static GcState {
    rustpython_common::static_cell! { static GC_STATE: GcState; }
    GC_STATE.get_or_init(GcState::new)
}

#[cfg(test)]
mod tests;

/// Access native collection bookkeeping.
///
/// # Safety
/// Tracking and untracking must obey the attached owner and heap lifetime rules
/// documented in [`crate::embedding`].
#[must_use]
pub unsafe fn gc_state_unchecked() -> &'static GcState {
    gc_state()
}

/// Access the current native allocation owner.
///
/// # Safety
/// Do not access this heap's objects outside their owner's attachment.
#[must_use]
pub unsafe fn current_owner_unchecked() -> PyRc<GcHeap> {
    current_owner()
}
