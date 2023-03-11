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
    fn enqueue(brc: &Brc<Self>, tid: Self::ThreadId);
}

// TODO: IMMORTAL & DEFERRED
const BIASED_SHIFT: i32 = 0;

const SHARED_SHIFT: i32 = 2;
const SHARED_FLAG_MERGED: i32 = 1;
const SHARED_FLAG_QUEUED: i32 = 2;

impl<Op: BrcThreadOp> Brc<Op> {
    pub fn increment(&self) {
        if self.is_local_thread() {
            self.fast_increment();
        } else {
            self.slow_increment();
        }
    }

    pub fn decrement(&self) {
        if self.is_local_thread() {
            self.fast_decrement();
        } else {
            self.slow_decrement();
        }
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

        // still alive
        if rc != 0 {
            return false;
        }

        // local ref reached zero
        // release the tid
        self.tid.set(None);
        // set merged flag
        let old = self.shared.fetch_or(SHARED_FLAG_MERGED, Ordering::SeqCst);
        // if queued flag not set, free to dealloc
        old & !SHARED_FLAG_QUEUED == 0
    }

    #[cold]
    fn slow_decrement(&self) -> bool {
        // We need to grab the thread-id before modifying the refcount
        // because the owning thread may set it to zero if we mark the
        // object as queued.
        let tid = self.tid.get().expect("tid is None on slow_decrement()");
        let mut queue;
        let mut new;

        loop {
            let old = self.shared.load(Ordering::Relaxed);

            queue = old == 0;
            new = if queue {
                // If the object had refcount zero, not queued, and not merged,
                // then we enqueue the object to be merged by the owning thread.
                // TODO: we should subtract one either here or where queue the object
                old | SHARED_FLAG_QUEUED
            } else {
                // Otherwise, subtract one from the reference count. This might
                // be negative!
                old - (1 << SHARED_SHIFT)
            };

            if let Ok(_) =
                self.shared
                    .compare_exchange(old, new, Ordering::SeqCst, Ordering::SeqCst)
            {
                break;
            }
        }

        if queue {
            // TODO: queue object
            Op::enqueue(self, tid);
            false
        } else if is_merged(new) && (new >> SHARED_SHIFT) == 0 {
            true
        } else {
            false
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
