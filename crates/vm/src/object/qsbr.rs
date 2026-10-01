//! Quiescent-state-based reclamation (QSBR) for lock-free caches.
//!
//! Objects published to lock-free caches (type method cache, type
//! specialization caches) are read via borrowed pointers plus try-incref.
//! Their memory must stay mapped until every thread that could hold such a
//! borrowed pointer has passed a quiescent state. Destructors run at the
//! normal drop point; only the final deallocation is deferred.
//!
//! Mirrors _Py_qsbr (Python/qsbr.c): a domain's write sequence advances on
//! each retirement; each thread records the last sequence it observed at a
//! quiescent point (eval-breaker checkpoint, attach/detach). A retired
//! allocation is freed once every online thread's sequence passes its goal.
//! Each interpreter has a domain; immutable shared definitions use a separate
//! process-wide domain. A busy interpreter cannot retain another one's memory.

use core::alloc::Layout;

/// Sequence value of an offline (detached) thread.
#[cfg(feature = "threading")]
const QSBR_OFFLINE: u64 = 0;
/// Initial write sequence value.
#[cfg(feature = "threading")]
const QSBR_INITIAL: u64 = 1;
/// Write sequence increment.
#[cfg(feature = "threading")]
const QSBR_INCR: u64 = 2;

#[cfg(feature = "threading")]
pub(crate) use threading::*;

#[cfg(feature = "threading")]
mod threading {
    use super::*;
    use alloc::sync::{Arc, Weak};
    use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::Mutex;

    /// Per-thread QSBR state, owned by the thread's `ThreadSlot`.
    pub(crate) struct QsbrSlot {
        domain: Arc<Qsbr>,
        /// Last write sequence observed at a quiescent point;
        /// `QSBR_OFFLINE` while the thread is detached.
        seq: AtomicU64,
        /// Set when this thread should pass a checkpoint (eval-breaker bit).
        pub(crate) requested: AtomicBool,
    }

    impl QsbrSlot {
        pub(crate) fn online(&self) {
            self.domain.online(self);
        }

        pub(crate) fn offline(&self) {
            self.domain.offline(self);
            self.domain.process();
        }

        pub(crate) fn checkpoint(&self) {
            self.requested.store(false, Ordering::Relaxed);
            self.domain.quiescent_state(self);
            self.domain.process();
        }
    }

    impl Drop for QsbrSlot {
        fn drop(&mut self) {
            // A registry scan can release the last upgraded slot while holding
            // the threads lock. Do not recursively process that registry here.
            self.domain.offline(self);
        }
    }

    #[cfg(test)]
    impl QsbrSlot {
        pub(crate) fn is_offline(&self) -> bool {
            self.seq.load(Ordering::Acquire) == QSBR_OFFLINE
        }
    }

    struct Retired {
        ptr: *mut u8,
        layout: Layout,
        goal: u64,
    }
    // SAFETY: `ptr` is an exclusively owned dead allocation; only the
    // processing thread touches it.
    unsafe impl Send for Retired {}

    pub(crate) struct Qsbr {
        shared: bool,
        /// Global write sequence (_Py_qsbr wr_seq).
        wr_seq: AtomicU64,
        /// Cached minimum observed read sequence (rd_seq).
        rd_seq: AtomicU64,
        threads: Mutex<Vec<Weak<QsbrSlot>>>,
        queue: Mutex<Vec<Retired>>,
        /// Set while the retire queue is non-empty; gates the per-instruction
        /// eval-breaker check so the hot path pays only one relaxed static
        /// load when nothing is pending.
        pending: AtomicBool,
    }

    pub(crate) fn shared() -> &'static Arc<Qsbr> {
        static SHARED: std::sync::OnceLock<Arc<Qsbr>> = std::sync::OnceLock::new();
        SHARED.get_or_init(|| {
            let mut domain = Qsbr::new();
            domain.shared = true;
            Arc::new(domain)
        })
    }

    impl Qsbr {
        pub(crate) const fn new() -> Self {
            Self {
                shared: false,
                wr_seq: AtomicU64::new(QSBR_INITIAL),
                rd_seq: AtomicU64::new(QSBR_INITIAL),
                threads: Mutex::new(Vec::new()),
                queue: Mutex::new(Vec::new()),
                pending: AtomicBool::new(false),
            }
        }

        /// Whether this domain has allocations awaiting a grace period.
        #[inline]
        pub(crate) fn break_pending(&self) -> bool {
            self.pending.load(Ordering::Relaxed)
        }

        /// Mirror `pending` into the global eval-breaker word — only for
        /// the global QSBR instance, so unit-test instances never touch
        /// process-global state.
        fn update_breaker_bit(&self, on: bool) {
            if self.shared {
                if on {
                    crate::signal::set_qsbr_bit();
                } else {
                    crate::signal::clear_qsbr_bit();
                }
            }
        }

        /// Register a detached thread. It must go online before reading any
        /// cache pointer. Dropping the slot unregisters the thread.
        pub(crate) fn register(self: &Arc<Self>) -> Arc<QsbrSlot> {
            let slot = Arc::new(QsbrSlot {
                domain: self.clone(),
                seq: AtomicU64::new(QSBR_OFFLINE),
                requested: AtomicBool::new(false),
            });
            self.threads.lock().unwrap().push(Arc::downgrade(&slot));
            slot
        }

        /// Advance the write sequence; returns the goal a retirement must
        /// wait for (_Py_qsbr_advance).
        fn advance(&self) -> u64 {
            self.wr_seq.fetch_add(QSBR_INCR, Ordering::AcqRel) + QSBR_INCR
        }

        /// Record that the calling thread is at a quiescent point: it holds
        /// no borrowed cache pointers (_Py_qsbr_quiescent_state).
        pub(crate) fn quiescent_state(&self, slot: &QsbrSlot) {
            slot.seq
                .store(self.wr_seq.load(Ordering::Acquire), Ordering::Release);
        }

        /// Mark a thread offline (detached); it no longer delays grace
        /// periods (_Py_qsbr_detach). The thread must not perform lock-free
        /// cache reads while offline.
        pub(crate) fn offline(&self, slot: &QsbrSlot) {
            slot.seq.store(QSBR_OFFLINE, Ordering::Release);
        }

        /// Mark a thread online again (_Py_qsbr_attach).
        pub(crate) fn online(&self, slot: &QsbrSlot) {
            // Order publication before the first borrowed cache read. A
            // release store alone allows the reader to race past a scan that
            // still sees OFFLINE; pair with poll_scan's SeqCst fence.
            slot.seq
                .store(self.wr_seq.load(Ordering::Acquire), Ordering::SeqCst);
        }

        /// Whether every online thread has passed `goal` (_Py_qsbr_poll).
        fn poll(&self, goal: u64) -> bool {
            if self.rd_seq.load(Ordering::Acquire) >= goal {
                return true;
            }
            self.poll_scan() >= goal
        }

        /// Recompute the minimum sequence over all live online threads,
        /// pruning dead ones.
        fn poll_scan(&self) -> u64 {
            core::sync::atomic::fence(Ordering::SeqCst);
            let mut min_seq = self.wr_seq.load(Ordering::Acquire);
            let mut threads = self.threads.lock().unwrap();
            threads.retain(|weak| match weak.upgrade() {
                Some(slot) => {
                    let seq = slot.seq.load(Ordering::Acquire);
                    if seq != QSBR_OFFLINE {
                        min_seq = min_seq.min(seq);
                    }
                    true
                }
                None => false,
            });
            drop(threads);
            self.rd_seq.fetch_max(min_seq, Ordering::AcqRel);
            min_seq
        }

        /// Defer deallocation of a dead object's memory until a grace
        /// period passes (_PyMem_FreeDelayed).
        ///
        /// # Safety
        /// `ptr`/`layout` must describe an allocation whose contents have
        /// been dropped and which nothing accesses afterwards except the
        /// racing try-incref reads this mechanism protects against.
        pub(crate) unsafe fn free_delayed(&self, ptr: *mut u8, layout: Layout) {
            let goal = self.advance();
            {
                let mut queue = self.queue.lock().unwrap();
                queue.push(Retired { ptr, layout, goal });
                // Set while still holding the queue lock, so this pairs with
                // `process` clearing the flag under the same lock and no
                // push can be left behind with the flag cleared.
                self.pending.store(true, Ordering::Release);
                self.update_breaker_bit(true);
            }
            // Ask every registered thread to pass a checkpoint.
            for weak in self.threads.lock().unwrap().iter() {
                if let Some(slot) = weak.upgrade() {
                    slot.requested.store(true, Ordering::Release);
                }
            }
        }

        /// Free retired allocations whose grace period has passed
        /// (_PyMem_ProcessDelayed).
        pub(crate) fn process(&self) {
            let Ok(mut queue) = self.queue.try_lock() else {
                // Another thread is already processing.
                return;
            };
            // Goals are usually increasing in push order, but concurrent
            // `free_delayed` calls can interleave their `advance()` and
            // queue push, so a smaller goal can occasionally land behind a
            // larger one. Free the longest prefix whose grace period has
            // passed; each drained item individually passed `poll`, so this
            // is sound regardless of ordering. A goal stuck behind an
            // out-of-order neighbor just waits for the next checkpoint or
            // GC pass, not a correctness issue.
            let safe_prefix = queue
                .iter()
                .position(|item| !self.poll(item.goal))
                .unwrap_or(queue.len());
            for item in queue.drain(..safe_prefix) {
                // SAFETY: grace period passed; no reader can hold `ptr`.
                unsafe { alloc::alloc::dealloc(item.ptr, item.layout) };
            }
            if queue.is_empty() {
                self.pending.store(false, Ordering::Release);
                self.update_breaker_bit(false);
            }
        }

        /// Stabilize queue and reader registry before the syscall, in the same
        /// lock order as process(). Detached native retirement also uses these
        /// mutexes, so stopping Python execution alone is insufficient.
        #[cfg(unix)]
        pub(crate) fn prepare_fork(&self) -> ForkDomain<'_> {
            ForkDomain {
                domain: self,
                queue: self.queue.lock().unwrap(),
                threads: self.threads.lock().unwrap(),
            }
        }

        #[cfg(test)]
        fn pending(&self) -> usize {
            self.queue.lock().unwrap().len()
        }
    }

    #[cfg(unix)]
    pub(crate) struct ForkDomain<'a> {
        domain: &'a Qsbr,
        queue: std::sync::MutexGuard<'a, Vec<Retired>>,
        threads: std::sync::MutexGuard<'a, Vec<Weak<QsbrSlot>>>,
    }

    #[cfg(unix)]
    impl ForkDomain<'_> {
        /// # Safety
        /// Only the fork child may run this, with no borrowed cache pointer on
        /// its native stack. `surviving` contains only this thread's slots,
        /// including detached slots of saved/nested interpreter entries.
        pub(crate) unsafe fn repair_child(&mut self, surviving: &[Arc<QsbrSlot>]) {
            self.threads.retain(|weak| {
                surviving
                    .iter()
                    .any(|slot| core::ptr::eq(weak.as_ptr(), Arc::as_ptr(slot)))
            });
            for weak in self.threads.iter() {
                if let Some(slot) = weak.upgrade() {
                    if slot.seq.load(Ordering::Relaxed) != QSBR_OFFLINE {
                        self.domain.quiescent_state(&slot);
                    }
                    slot.requested.store(false, Ordering::Relaxed);
                }
            }
            for item in self.queue.drain(..) {
                unsafe { alloc::alloc::dealloc(item.ptr, item.layout) };
            }
            self.domain.pending.store(false, Ordering::Release);
            self.domain.update_breaker_bit(false);
        }
    }

    impl Drop for Qsbr {
        fn drop(&mut self) {
            // Every registered slot retains the domain. Its last owner can
            // therefore release queued allocations without waiting for a VM.
            let queue = self
                .queue
                .get_mut()
                .unwrap_or_else(|error| error.into_inner());
            for item in queue.drain(..) {
                unsafe { alloc::alloc::dealloc(item.ptr, item.layout) };
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn poll_requires_all_online_threads() {
            let q = Arc::new(Qsbr::new());
            let a = q.register();
            let b = q.register();
            a.online();
            b.online();
            let goal = q.advance();
            assert!(!q.poll(goal));
            q.quiescent_state(&a);
            assert!(!q.poll(goal));
            q.quiescent_state(&b);
            assert!(q.poll(goal));
        }

        #[test]
        fn offline_thread_does_not_delay_grace() {
            let q = Arc::new(Qsbr::new());
            let a = q.register();
            let b = q.register();
            a.online();
            b.online();
            let goal = q.advance();
            q.quiescent_state(&a);
            q.offline(&b);
            assert!(q.poll(goal));
        }

        #[test]
        fn dead_thread_is_pruned() {
            let q = Arc::new(Qsbr::new());
            let a = q.register();
            let b = q.register();
            a.online();
            b.online();
            drop(b);
            let goal = q.advance();
            q.quiescent_state(&a);
            assert!(q.poll(goal));
        }

        #[test]
        fn process_frees_only_after_grace() {
            let q = Arc::new(Qsbr::new());
            let a = q.register();
            a.online();
            let layout = Layout::new::<u64>();
            let ptr = unsafe { alloc::alloc::alloc(layout) };
            unsafe { q.free_delayed(ptr, layout) };
            assert!(a.requested.load(Ordering::Acquire));
            assert!(q.break_pending());
            q.process();
            assert_eq!(q.pending(), 1); // grace period not passed yet
            assert!(q.break_pending());
            q.quiescent_state(&a);
            q.process();
            assert_eq!(q.pending(), 0);
            assert!(!q.break_pending());
        }

        #[test]
        fn domains_do_not_wait_for_each_others_readers() {
            let first = Arc::new(Qsbr::new());
            let second = Arc::new(Qsbr::new());
            let a = first.register();
            let b = second.register();
            a.online();
            b.online();
            let layout = Layout::new::<u64>();
            for domain in [&first, &second] {
                let ptr = unsafe { alloc::alloc::alloc(layout) };
                unsafe { domain.free_delayed(ptr, layout) };
            }
            b.checkpoint();
            assert_eq!(second.pending(), 0);
            assert_eq!(first.pending(), 1);
            a.offline();
            assert_eq!(first.pending(), 0);
        }

        #[test]
        fn registered_but_unattached_threads_do_not_retain_memory() {
            let domain = Arc::new(Qsbr::new());
            let slot = domain.register();
            assert!(slot.is_offline());
            let layout = Layout::new::<u64>();
            let ptr = unsafe { alloc::alloc::alloc(layout) };
            unsafe { domain.free_delayed(ptr, layout) };
            domain.process();
            assert_eq!(domain.pending(), 0);
        }

        #[test]
        fn last_reader_keeps_its_domain_alive() {
            let domain = Arc::new(Qsbr::new());
            let reader = domain.register();
            reader.online();
            let weak = Arc::downgrade(&domain);
            let layout = Layout::new::<u64>();
            let ptr = unsafe { alloc::alloc::alloc(layout) };
            unsafe { domain.free_delayed(ptr, layout) };
            drop(domain);
            assert!(weak.upgrade().is_some());
            reader.checkpoint();
            assert_eq!(weak.upgrade().unwrap().pending(), 0);
            drop(reader);
            assert!(weak.upgrade().is_none());
        }
    }
}

/// Retain the allocation's reclamation domain before dropping its object
/// header. The current VM can be unrelated to the allocation's owner.
pub(crate) struct RetireDomain {
    #[cfg(feature = "threading")]
    domain: alloc::sync::Arc<Qsbr>,
}

impl RetireDomain {
    pub(crate) fn for_object(object: &crate::PyObject) -> Self {
        #[cfg(not(feature = "threading"))]
        let _ = object;
        Self {
            #[cfg(feature = "threading")]
            domain: object
                .gc_handle()
                .heap()
                .map_or_else(|| shared().clone(), |heap| heap.qsbr.clone()),
        }
    }

    /// # Safety
    /// The allocation is dead; only pending pre-try-incref readers may touch it.
    pub(crate) unsafe fn free(self, ptr: *mut u8, layout: Layout) {
        #[cfg(feature = "threading")]
        unsafe {
            self.domain.free_delayed(ptr, layout)
        };
        #[cfg(not(feature = "threading"))]
        unsafe {
            alloc::alloc::dealloc(ptr, layout)
        };
    }
}
