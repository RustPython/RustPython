use crate::atomic::{Ordering::*, PyAtomic, Radium};

/// from alloc::sync
/// A soft limit on the amount of references that may be made to an `Arc`.
///
/// Going above this limit will abort your program (although not
/// necessarily) at _exactly_ `MAX_REFCOUNT + 1` references.
const MAX_REFCOUNT: usize = (isize::MAX) as usize;

pub struct RefCount {
    strong: PyAtomic<usize>,
}

impl Default for RefCount {
    fn default() -> Self {
        Self::new()
    }
}

impl RefCount {
    pub fn new() -> Self {
        RefCount {
            strong: Radium::new(1),
        }
    }

    #[inline]
    pub fn get(&self) -> usize {
        self.strong.load(SeqCst)
    }

    #[inline]
    pub fn inc(&self) {
        let old_size = self.strong.fetch_add(1, Relaxed);

        if old_size > MAX_REFCOUNT {
            std::process::abort();
        }
    }

    /// Returns true if successful
    #[inline]
    pub fn safe_inc(&self) -> bool {
        self.strong
            .fetch_update(AcqRel, Acquire, |prev| (prev != 0).then(|| prev + 1))
            .is_ok()
    }

    /// Decrement the reference count. Returns true when the refcount drops to 0.
    #[inline]
    pub fn dec(&self) -> bool {
        if self.strong.fetch_sub(1, Release) != 1 {
            return false;
        }

        PyAtomic::<usize>::fence(Acquire);

        true
    }
}
