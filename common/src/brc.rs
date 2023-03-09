use std::{
    cell::Cell,
    sync::atomic::{AtomicI32, Ordering},
    thread::ThreadId,
};

pub struct Brc {
    tid: Cell<Option<ThreadId>>,
    biased: Cell<u32>,
    shared: AtomicI32,
}

// TODO: IMMORTAL & DEFERRED
const BIASED_SHIFT: i32 = 0;

const SHARED_SHIFT: i32 = 2;
const SHARED_FLAG_MERGED: i32 = 1;
const SHARED_FLAG_QUEUED: i32 = 2;

impl Brc {
    fn fast_increment(&self) {
        let mut rc = self.biased.get();
        rc += 1 << BIASED_SHIFT;
        self.biased.set(rc);
    }
    #[cold]
    fn slow_increment(&self) {
        self.shared.fetch_add(1 << SHARED_SHIFT, Ordering::Relaxed);
    }
    fn increment(&self) {
        if self.tid.get() == Some(std::thread::current().id()) {
            self.fast_increment();
        } else {
            self.slow_increment();
        }
    }

    fn fast_decrement(&self) -> bool {
        let mut rc = self.biased.get();
        rc -= 1 << BIASED_SHIFT;
        self.biased.set(rc);
        if rc != 0 {
            return false;
        }
        let old = self.shared.fetch_or(SHARED_FLAG_MERGED, Ordering::Relaxed);
        self.tid.set(None);
        old & !SHARED_FLAG_QUEUED == 0
    }

    fn slow_dec(&self) -> bool {
        loop {
            let old = self.shared.load(Ordering::SeqCst);
        }
    }
}
