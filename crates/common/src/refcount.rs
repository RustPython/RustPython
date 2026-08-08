use crate::atomic::{Ordering, PyAtomic, Radium};

// State layout (usize):
//   [1 bit: destructed] [1 bit: published] [1 bit: leaked] [M bits: strong_count]
// 64-bit: M=61.  32-bit: M=29.
//
// Weak references live in the object's `WeakRefList`, not in this word, so the
// strong count takes every bit the flags leave. A 32-bit target reaches its
// ceiling at 536 870 911 references rather than the 32 767 that half the word
// would allow — a number two ordinary module imports pass on `wasm32`.
const FLAG_BITS: u32 = 3;
const DESTRUCTED: usize = 1 << (usize::BITS - 1);
/// Object was published to a lock-free cache; memory reclamation is
/// deferred through QSBR so concurrent try-incref readers never touch
/// freed memory. Sticky once set.
const PUBLISHED: usize = 1 << (usize::BITS - 2);
const LEAKED: usize = 1 << (usize::BITS - 3);
const STRONG_WIDTH: u32 = usize::BITS - FLAG_BITS;
const STRONG: usize = (1 << STRONG_WIDTH) - 1;
const COUNT: usize = 1;

#[inline(never)]
#[cold]
#[allow(
    clippy::disallowed_methods,
    reason = "refcount overflow must preserve upstream abort semantics"
)]
fn refcount_overflow() -> ! {
    cfg_select! {
        feature = "std" => std::process::abort(),
        _ => core::panic!("refcount overflow"),
    }
}

/// State wraps reference count + flags in a single word (platform usize)
#[derive(Clone, Copy)]
struct State {
    inner: usize,
}

impl State {
    #[inline]
    fn from_raw(inner: usize) -> Self {
        Self { inner }
    }

    #[inline]
    fn as_raw(self) -> usize {
        self.inner
    }

    #[inline]
    fn strong(self) -> usize {
        (self.inner & STRONG) / COUNT
    }

    #[inline]
    fn destructed(self) -> bool {
        (self.inner & DESTRUCTED) != 0
    }

    #[inline]
    fn leaked(self) -> bool {
        (self.inner & LEAKED) != 0
    }

    #[inline]
    fn add_strong(self, val: u32) -> Self {
        Self::from_raw(self.inner + (val as usize) * COUNT)
    }

    #[inline]
    fn with_leaked(self, leaked: bool) -> Self {
        Self::from_raw((self.inner & !LEAKED) | if leaked { LEAKED } else { 0 })
    }
}

/// Reference count using state layout with LEAKED support.
///
/// State layout (usize):
/// 64-bit: [1 bit: destructed] [1 bit: published] [1 bit: leaked] [61 bits: strong_count]
/// 32-bit: [1 bit: destructed] [1 bit: published] [1 bit: leaked] [29 bits: strong_count]
pub struct RefCount {
    state: PyAtomic<usize>,
}

impl Default for RefCount {
    fn default() -> Self {
        Self::new()
    }
}

impl RefCount {
    /// Create a new RefCount with strong count = 1
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: Radium::new(COUNT),
        }
    }

    /// Get current strong count
    #[inline]
    pub fn get(&self) -> usize {
        State::from_raw(self.state.load(Ordering::Relaxed)).strong()
    }

    /// Increment strong count
    #[inline]
    pub fn inc(&self) {
        let val = State::from_raw(self.state.fetch_add(COUNT, Ordering::Relaxed));
        if val.destructed() || val.strong() > STRONG - 1 {
            refcount_overflow();
        }
        if val.strong() == 0 {
            // The previous fetch_add created a permission to run decrement again
            self.state.fetch_add(COUNT, Ordering::Relaxed);
        }
    }

    #[inline]
    pub fn inc_by(&self, n: usize) {
        debug_assert!(n <= STRONG);
        let val = State::from_raw(self.state.fetch_add(n * COUNT, Ordering::Relaxed));
        if val.destructed() || val.strong() > STRONG - n {
            refcount_overflow();
        }
    }

    /// Returns true if successful
    #[inline]
    #[must_use]
    pub fn safe_inc(&self) -> bool {
        let mut old = State::from_raw(self.state.load(Ordering::Relaxed));
        loop {
            if old.destructed() || old.strong() == 0 {
                return false;
            }
            if old.strong() >= STRONG {
                refcount_overflow();
            }
            let new_state = old.add_strong(1);
            match self.state.compare_exchange_weak(
                old.as_raw(),
                new_state.as_raw(),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return true,
                Err(curr) => old = State::from_raw(curr),
            }
        }
    }

    /// Decrement strong count. Returns true when count drops to 0.
    #[inline]
    #[must_use]
    pub fn dec(&self) -> bool {
        let old = State::from_raw(self.state.fetch_sub(COUNT, Ordering::Release));

        // LEAKED objects never reach 0
        if old.leaked() {
            return false;
        }

        if old.strong() == 1 {
            core::sync::atomic::fence(Ordering::Acquire);
            return true;
        }
        false
    }

    /// Mark this object as leaked (interned). It will never be deallocated.
    pub fn leak(&self) {
        debug_assert!(!self.is_leaked());
        let mut old = State::from_raw(self.state.load(Ordering::Relaxed));
        loop {
            let new_state = old.with_leaked(true);
            match self.state.compare_exchange_weak(
                old.as_raw(),
                new_state.as_raw(),
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => return,
                Err(curr) => old = State::from_raw(curr),
            }
        }
    }

    /// Check if this object is leaked (interned).
    pub fn is_leaked(&self) -> bool {
        State::from_raw(self.state.load(Ordering::Acquire)).leaked()
    }

    /// Mark the object as published to a lock-free cache (sticky).
    #[inline]
    pub fn mark_published(&self) {
        self.state.fetch_or(PUBLISHED, Ordering::Release);
    }

    #[inline]
    pub fn is_published(&self) -> bool {
        (self.state.load(Ordering::Acquire) & PUBLISHED) != 0
    }
}

// Deferred Drop Infrastructure
//
// This mechanism allows untrack_object() calls to be deferred until after
// the GC collection phase completes, preventing deadlocks that occur when
// clear (pop_edges) triggers object destruction while holding the tracked_objects lock.

#[cfg(feature = "std")]
use core::cell::{Cell, RefCell};

#[cfg(feature = "std")]
thread_local! {
    /// Flag indicating if we're inside a deferred drop context.
    /// When true, drop operations should defer untrack calls.
    static IN_DEFERRED_CONTEXT: Cell<bool> = const { Cell::new(false) };

    /// Queue of deferred untrack operations.
    /// No Send bound needed - this is thread-local and only accessed from the same thread.
    static DEFERRED_QUEUE: RefCell<Vec<Box<dyn FnOnce()>>> = const { RefCell::new(Vec::new()) };
}

#[cfg(feature = "std")]
struct DeferredDropGuard {
    was_in_context: bool,
}

#[cfg(feature = "std")]
impl Drop for DeferredDropGuard {
    fn drop(&mut self) {
        IN_DEFERRED_CONTEXT.with(|in_ctx| {
            in_ctx.set(self.was_in_context);
        });
        // Only flush if we're the outermost context and not already panicking
        // (flushing during unwinding risks double-panic → process abort).
        if !self.was_in_context && !std::thread::panicking() {
            flush_deferred_drops();
        }
    }
}

/// Execute a function within a deferred drop context.
/// Any calls to `try_defer_drop` within this context will be queued
/// and executed when the context exits (even on panic).
#[cfg(feature = "std")]
#[inline]
pub fn with_deferred_drops<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    let _guard = IN_DEFERRED_CONTEXT.with(|in_ctx| {
        let was_in_context = in_ctx.get();
        in_ctx.set(true);
        DeferredDropGuard { was_in_context }
    });
    f()
}

/// Try to defer a drop-related operation.
/// If inside a deferred context, the operation is queued.
/// Otherwise, it executes immediately.
#[cfg(feature = "std")]
#[inline]
pub fn try_defer_drop<F>(f: F)
where
    F: FnOnce() + 'static,
{
    let should_defer = IN_DEFERRED_CONTEXT.with(|in_ctx| in_ctx.get());

    if should_defer {
        DEFERRED_QUEUE.with(|q| {
            q.borrow_mut().push(Box::new(f));
        });
    } else {
        f();
    }
}

/// Flush all deferred drop operations.
/// This is automatically called when exiting a deferred context.
#[cfg(feature = "std")]
#[inline]
pub fn flush_deferred_drops() {
    DEFERRED_QUEUE.with(|q| {
        // Take all queued operations
        let ops: Vec<_> = q.borrow_mut().drain(..).collect();
        // Execute them outside the borrow
        for op in ops {
            op();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The strong count reaches far past a 16-bit ceiling on every target.
    ///
    /// The count shares its word with the flag bits, so its width follows the
    /// pointer width. A 32-bit target is the one this guards: a second counter
    /// packed beside the strong count once left it 15 bits, and `wasm32`
    /// aborted at 32 767 references — a total two ordinary module imports
    /// pass. The check is a no-op on a 64-bit host, where 31 bits already
    /// covered this; run the crate's tests against a 32-bit target to exercise
    /// it.
    #[test]
    fn strong_count_reaches_past_a_16_bit_ceiling() {
        const REFERENCES: usize = 1 << 20;

        let rc = RefCount::new();
        rc.inc_by(REFERENCES);
        assert_eq!(rc.get(), REFERENCES + 1);
    }

    /// A fresh count holds exactly one strong reference and no stray bits.
    ///
    /// `get` masks the flags away, so a spare field left in the word would not
    /// show up there. Reading the raw state keeps the layout honest.
    #[test]
    fn a_new_refcount_holds_one_strong_reference_and_nothing_else() {
        let rc = RefCount::new();
        assert_eq!(rc.get(), 1);
        assert_eq!(rc.state.load(Ordering::Relaxed), COUNT);
    }

    #[test]
    fn published_bit_survives_refcount_traffic() {
        let rc = RefCount::new(); // strong = 1
        assert!(!rc.is_published());
        rc.mark_published();
        assert!(rc.is_published());
        rc.inc(); // strong = 2
        assert!(rc.is_published());
        assert!(!rc.dec()); // strong = 1
        assert!(rc.is_published());
        assert!(rc.safe_inc()); // strong = 2
        assert!(!rc.dec()); // strong = 1
        assert!(rc.dec()); // strong = 0 -> true
        assert!(rc.is_published());
    }
}
