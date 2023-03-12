// REFERENCES
// https://github.com/colesbury/nogil
// https://iacoma.cs.uiuc.edu/iacoma-papers/pact18.pdf

use std::{
    cell::Cell,
    sync::atomic::{AtomicI32, Ordering},
};

pub struct Brc<Op: BrcThreadOp + ?Sized> {
    tid: Cell<Option<Op::ThreadId>>,
    biased: Cell<u32>,
    shared: AtomicI32,
}

pub trait BrcThreadOp {
    type ThreadId: Copy + Eq;
    fn current_thread_id() -> Self::ThreadId;
    #[cold]
    fn enqueue(brc: &Brc<Self>, tid: Self::ThreadId);
}

impl<Op: BrcThreadOp> Default for Brc<Op> {
    fn default() -> Self {
        Self::new()
    }
}

unsafe impl<Op: BrcThreadOp> Send for Brc<Op> {}
unsafe impl<Op: BrcThreadOp> Sync for Brc<Op> {}

// TODO: IMMORTAL & DEFERRED
const BIASED_SHIFT: i32 = 0;

const SHARED_SHIFT: i32 = 2;
const SHARED_FLAG_MERGED: i32 = 1;
const SHARED_FLAG_QUEUED: i32 = 2;

impl<Op: BrcThreadOp> Brc<Op> {
    pub fn new() -> Self {
        Self {
            tid: Cell::new(Some(Op::current_thread_id())),
            biased: Cell::new(1),
            shared: AtomicI32::new(0),
        }
    }

    pub fn inc(&self) {
        if self.is_local_thread() {
            self.fast_increment();
        } else {
            self.slow_increment();
        }
    }

    pub fn dec(&self) -> bool {
        if self.is_local_thread() {
            self.fast_decrement()
        } else {
            self.slow_decrement()
        }
    }

    pub fn safe_inc(&self) -> bool {
        // TODO: DEFERRED?
        self.inc();
        true
    }

    pub fn get(&self) -> usize {
        ((self.biased.get() >> BIASED_SHIFT)
            + (self.shared.load(Ordering::SeqCst) >> SHARED_SHIFT) as u32) as usize
    }

    pub fn leak(&self) {
        // TODO: IMMORTAL?
        self.biased.set(self.biased.get() + (1 << 31))
    }

    pub fn is_leaked(&self) -> bool {
        self.biased.get() & (1 << 31) != 0
    }

    fn fast_increment(&self) {
        let mut rc = self.biased.get();
        rc += 1 << BIASED_SHIFT;
        self.biased.set(rc);
    }

    #[cold]
    fn slow_increment(&self) {
        self.shared.fetch_add(1 << SHARED_SHIFT, Ordering::SeqCst);
    }

    fn fast_decrement(&self) -> bool {
        let mut rc = self.biased.get();
        rc -= 1 << BIASED_SHIFT;
        self.biased.set(rc);

        // still alive?
        if rc != 0 {
            return false;
        }

        // set merged flag
        let shared = self.shared.fetch_or(SHARED_FLAG_MERGED, Ordering::SeqCst);
        // release the tid
        self.tid.set(None);
        // free to dealloc if shared count is zero
        shared_count(shared) == 0
    }

    #[cold]
    fn slow_decrement(&self) -> bool {
        // We need to grab the thread-id before modifying the refcount
        // because the owning thread may set it to zero if we mark the
        // object as queued.
        let tid = self.tid.get();
        let mut queue;
        let mut shared;

        loop {
            let old = self.shared.load(Ordering::Relaxed);

            queue = old == 0;
            shared = if queue {
                // If the object had refcount zero, not queued, and not merged,
                // then we enqueue the object to be merged by the owning thread.
                // TODO: we should subtract one either here or where queue the object
                old | SHARED_FLAG_QUEUED
            } else {
                // Otherwise, subtract one from the reference count. This might
                // be negative!
                old - (1 << SHARED_SHIFT)
            };

            if self
                .shared
                .compare_exchange(old, shared, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                break;
            }
        }

        if queue {
            let tid = tid.expect("tid is None but try to queue the object");
            Op::enqueue(self, tid);
            false
        } else {
            is_merged(shared) && shared_count(shared) == 0
        }
    }

    fn is_local_thread(&self) -> bool {
        self.tid.get() == Some(Op::current_thread_id())
    }
}

fn is_merged(shared: i32) -> bool {
    shared & SHARED_FLAG_MERGED != 0
}

fn is_queued(shared: i32) -> bool {
    shared & SHARED_FLAG_QUEUED != 0
}

fn shared_count(shared: i32) -> i32 {
    shared >> SHARED_SHIFT
}

fn biased_count(biased: u32) -> u32 {
    biased >> BIASED_SHIFT
}
