//! Quiescent-state-based reclamation (QSBR) for lock-free caches.
//!
//! Objects published to lock-free caches (type method cache, type
//! specialization caches) are read via borrowed pointers plus try-incref.
//! Their memory must stay mapped until every thread that could hold such a
//! borrowed pointer has passed a quiescent state. Destructors run at the
//! normal drop point; only the final deallocation is deferred.
//!
//! Mirrors _Py_qsbr (Python/qsbr.c): a global write sequence advances on
//! each retirement; each thread records the last sequence it observed at a
//! quiescent point (eval-breaker checkpoint, attach/detach). A retired
//! allocation is freed once every online thread's sequence passes its goal.

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
        /// Last write sequence observed at a quiescent point;
        /// `QSBR_OFFLINE` while the thread is detached.
        seq: AtomicU64,
        /// Set when this thread should pass a checkpoint (eval-breaker bit).
        pub(crate) requested: AtomicBool,
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

    pub(crate) static QSBR: Qsbr = Qsbr::new();

    impl Qsbr {
        const fn new() -> Self {
            Self {
                wr_seq: AtomicU64::new(QSBR_INITIAL),
                rd_seq: AtomicU64::new(QSBR_INITIAL),
                threads: Mutex::new(Vec::new()),
                queue: Mutex::new(Vec::new()),
                pending: AtomicBool::new(false),
            }
        }

        /// Whether retired allocations are pending. The hot path now reads
        /// the mirrored bit in the eval-breaker word instead; this stays
        /// only for unit tests that exercise local, non-global instances.
        #[inline]
        #[cfg_attr(not(test), allow(dead_code))]
        pub(crate) fn break_pending(&self) -> bool {
            self.pending.load(Ordering::Relaxed)
        }

        /// Mirror `pending` into the global eval-breaker word — only for
        /// the global QSBR instance, so unit-test instances never touch
        /// process-global state.
        fn update_breaker_bit(&self, on: bool) {
            if core::ptr::eq(self, &QSBR) {
                if on {
                    crate::signal::set_qsbr_bit();
                } else {
                    crate::signal::clear_qsbr_bit();
                }
            }
        }

        /// Register the calling thread. The returned slot is stored in the
        /// thread's `ThreadSlot`; dropping it unregisters the thread.
        pub(crate) fn register(&self) -> Arc<QsbrSlot> {
            let slot = Arc::new(QsbrSlot {
                seq: AtomicU64::new(self.wr_seq.load(Ordering::Acquire)),
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
        #[cfg(any(unix, test))]
        pub(crate) fn offline(&self, slot: &QsbrSlot) {
            slot.seq.store(QSBR_OFFLINE, Ordering::Release);
        }

        /// Mark a thread online again (_Py_qsbr_attach).
        #[cfg(unix)]
        pub(crate) fn online(&self, slot: &QsbrSlot) {
            self.quiescent_state(slot);
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

        /// Free all retired allocations immediately.
        ///
        /// # Safety
        /// Only sound when no other thread can be mid-read: the post-fork
        /// child, or teardown after all threads exited.
        #[cfg(unix)]
        pub(crate) unsafe fn drain_all(&self) {
            let mut queue = self.queue.lock().unwrap();
            for item in queue.drain(..) {
                // SAFETY: guaranteed single-threaded by the caller.
                unsafe { alloc::alloc::dealloc(item.ptr, item.layout) };
            }
            self.pending.store(false, Ordering::Release);
            self.update_breaker_bit(false);
        }

        /// Reset after fork: drop all registered thread entries (dead
        /// parent threads' slots would otherwise stay online forever and
        /// stall every future grace period) and free all retired
        /// allocations.
        ///
        /// # Safety
        /// Only sound in the single-threaded post-fork child, before the
        /// surviving thread re-registers.
        #[cfg(unix)]
        pub(crate) unsafe fn reset_after_fork(&self) {
            self.threads.lock().unwrap().clear();
            // SAFETY: single-threaded child, no concurrent reader exists.
            unsafe { self.drain_all() };
        }

        #[cfg(test)]
        fn pending(&self) -> usize {
            self.queue.lock().unwrap().len()
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn poll_requires_all_online_threads() {
            let q = Qsbr::new();
            let a = q.register();
            let b = q.register();
            let goal = q.advance();
            assert!(!q.poll(goal));
            q.quiescent_state(&a);
            assert!(!q.poll(goal));
            q.quiescent_state(&b);
            assert!(q.poll(goal));
        }

        #[test]
        fn offline_thread_does_not_delay_grace() {
            let q = Qsbr::new();
            let a = q.register();
            let b = q.register();
            let goal = q.advance();
            q.quiescent_state(&a);
            q.offline(&b);
            assert!(q.poll(goal));
        }

        #[test]
        fn dead_thread_is_pruned() {
            let q = Qsbr::new();
            let a = q.register();
            let b = q.register();
            drop(b);
            let goal = q.advance();
            q.quiescent_state(&a);
            assert!(q.poll(goal));
        }

        #[test]
        fn process_frees_only_after_grace() {
            let q = Qsbr::new();
            let a = q.register();
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
    }
}

/// Defer (threading) or immediately perform (non-threading) deallocation
/// of a dead published object's memory.
///
/// # Safety
/// Same contract as [`Qsbr::free_delayed`].
#[inline]
pub(crate) unsafe fn free_delayed(ptr: *mut u8, layout: Layout) {
    #[cfg(feature = "threading")]
    unsafe {
        QSBR.free_delayed(ptr, layout)
    };
    #[cfg(not(feature = "threading"))]
    // No concurrent readers can exist without threads.
    unsafe {
        alloc::alloc::dealloc(ptr, layout)
    };
}
