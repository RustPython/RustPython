// REFERENCES
// https://github.com/colesbury/nogil
// https://iacoma.cs.uiuc.edu/iacoma-papers/pact18.pdf

use std::{
    cell::Cell,
    sync::atomic::{AtomicI32, Ordering},
};

pub struct Brc<Op: BrcOp + ?Sized> {
    tid: Cell<Option<Op::ThreadId>>,
    biased: Cell<u32>,
    shared: AtomicI32,
}

pub trait BrcOp {
    type ThreadId: Copy + Eq;
    fn current_thread_id() -> Self::ThreadId;
    #[cold]
    fn enqueue(brc: &Brc<Self>, tid: Self::ThreadId);
    fn dealloc(brc: &Brc<Self>);
}

impl<Op: BrcOp> Default for Brc<Op> {
    fn default() -> Self {
        Self::new()
    }
}

unsafe impl<Op: BrcOp> Send for Brc<Op> {}
unsafe impl<Op: BrcOp> Sync for Brc<Op> {}

const BIASED_SHIFT: u32 = 1;
const BIASED_FLAG_IMMORTAL: u32 = 1;
// prevent double dealloc from __del__ slots
const BIASED_FLAG_DELETING: u32 = 1 << 31;

const SHARED_SHIFT: i32 = 2;
const SHARED_FLAG_MERGED: i32 = 1;
const SHARED_FLAG_QUEUED: i32 = 2;

impl<Op: BrcOp> Brc<Op> {
    pub fn new() -> Self {
        Self {
            tid: Cell::new(Some(Op::current_thread_id())),
            biased: Cell::new(1 << BIASED_SHIFT),
            shared: AtomicI32::new(0),
        }
    }

    pub fn inc(&self) {
        let biased = self.biased.get();

        if is_immortal(biased) {
            return;
        }

        if self.is_local_thread() {
            self.biased.set(biased + (1 << BIASED_SHIFT));
        } else {
            self.inc_shared();
        }
    }

    pub fn dec(&self) {
        let biased = self.biased.get();

        if is_immortal(biased) {
            return;
        }

        if self.is_local_thread() {
            let biased = biased - (1 << BIASED_SHIFT);
            self.biased.set(biased);

            if biased == 0 {
                self.merge_zero_biased();
            }
        } else {
            self.dec_shared()
        }
    }

    pub fn safe_inc(&self) -> bool {
        // TODO: DEFERRED?
        self.inc();
        true
    }

    pub fn get(&self) -> usize {
        (biased_count(self.biased.get()) + shared_count(self.shared.load(Ordering::Relaxed)) as u32)
            as usize
    }

    pub fn leak(&self) {
        self.biased.set(self.biased.get() | BIASED_FLAG_IMMORTAL);
    }

    pub fn is_leaked(&self) -> bool {
        is_immortal(self.biased.get())
    }

    pub unsafe fn enter_state_deleting(&self) {
        self.tid.set(Some(Op::current_thread_id()));
        self.biased.set((1 << BIASED_SHIFT) | BIASED_FLAG_DELETING);
        self.shared.store(0, Ordering::SeqCst);
    }

    pub unsafe fn leave_state_deleting(&self) {
        self.biased.set(self.biased.get() - BIASED_FLAG_DELETING - (1 << BIASED_SHIFT));
    }

    #[cold]
    fn inc_shared(&self) {
        self.shared.fetch_add(1 << SHARED_SHIFT, Ordering::SeqCst);
    }

    #[cold]
    fn dec_shared(&self) {
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
        } else if is_merged(shared) && shared_count(shared) == 0 {
            self.dealloc();
        }
    }

    fn is_local_thread(&self) -> bool {
        self.tid.get() == Some(Op::current_thread_id())
    }

    #[cold]
    fn merge_zero_biased(&self) {
        // set merged flag
        let shared = self.shared.fetch_or(SHARED_FLAG_MERGED, Ordering::SeqCst);
        // release the tid
        self.tid.set(None);
        // free to dealloc if shared count is zero
        if shared_count(shared) == 0 {
            self.dealloc();
        }
    }

    fn dealloc(&self) {
        Op::dealloc(self);
    }
}

fn is_merged(shared: i32) -> bool {
    shared & SHARED_FLAG_MERGED != 0
}

fn is_queued(shared: i32) -> bool {
    shared & SHARED_FLAG_QUEUED != 0
}

fn is_immortal(biased: u32) -> bool {
    biased & BIASED_FLAG_IMMORTAL != 0
}

fn shared_count(shared: i32) -> i32 {
    shared >> SHARED_SHIFT
}

fn biased_count(biased: u32) -> u32 {
    biased >> BIASED_SHIFT
}
