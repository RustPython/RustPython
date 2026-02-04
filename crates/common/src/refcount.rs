//! Reference counting implementation based on EBR (Epoch-Based Reclamation).

use crate::atomic::{Ordering, PyAtomic, Radium};
use std::cell::{Cell, RefCell};

pub use crate::epoch::Guard;

// State layout constants
const EPOCH_WIDTH: u32 = 0;
const EPOCH_MASK_HEIGHT: u32 = usize::BITS - EPOCH_WIDTH;
const DESTRUCTED: usize = 1 << (EPOCH_MASK_HEIGHT - 1);
const LEAKED: usize = 1 << (EPOCH_MASK_HEIGHT - 3);
const TOTAL_COUNT_WIDTH: u32 = usize::BITS - EPOCH_WIDTH - 3;
const WEAK_WIDTH: u32 = TOTAL_COUNT_WIDTH / 2;
const STRONG_WIDTH: u32 = TOTAL_COUNT_WIDTH - WEAK_WIDTH;
const STRONG: usize = (1 << STRONG_WIDTH) - 1;
const COUNT: usize = 1;
const WEAK_COUNT: usize = 1 << STRONG_WIDTH;

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
    fn strong(self) -> u32 {
        ((self.inner & STRONG) / COUNT) as u32
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
/// 64-bit: [1 bit: destructed] [1 bit: weaked] [1 bit: leaked] [30 bits: weak_count] [31 bits: strong_count]
/// 32-bit: [1 bit: destructed] [1 bit: weaked] [1 bit: leaked] [14 bits: weak_count] [15 bits: strong_count]
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
    pub fn new() -> Self {
        // Initial state: strong=1, weak=1 (implicit weak for strong refs)
        Self {
            state: Radium::new(COUNT + WEAK_COUNT),
        }
    }

    /// Get current strong count
    #[inline]
    pub fn get(&self) -> usize {
        State::from_raw(self.state.load(Ordering::SeqCst)).strong() as usize
    }

    /// Increment strong count
    #[inline]
    pub fn inc(&self) {
        let val = State::from_raw(self.state.fetch_add(COUNT, Ordering::SeqCst));
        if val.destructed() {
            // Already marked for destruction, but we're incrementing
            // This shouldn't happen in normal usage
            std::process::abort();
        }
        if val.strong() == 0 {
            // The previous fetch_add created a permission to run decrement again
            self.state.fetch_add(COUNT, Ordering::SeqCst);
        }
    }

    #[inline]
    pub fn inc_by(&self, n: usize) {
        debug_assert!(n <= STRONG);
        let val = State::from_raw(self.state.fetch_add(n * COUNT, Ordering::SeqCst));
        if val.destructed() || (val.strong() as usize) > STRONG - n {
            std::process::abort();
        }
    }

    /// Returns true if successful
    #[inline]
    pub fn safe_inc(&self) -> bool {
        let mut old = State::from_raw(self.state.load(Ordering::SeqCst));
        loop {
            if old.destructed() {
                return false;
            }
            let new_state = old.add_strong(1);
            match self.state.compare_exchange(
                old.as_raw(),
                new_state.as_raw(),
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => return true,
                Err(curr) => old = State::from_raw(curr),
            }
        }
    }

    /// Decrement strong count. Returns true when count drops to 0.
    #[inline]
    pub fn dec(&self) -> bool {
        let old = State::from_raw(self.state.fetch_sub(COUNT, Ordering::SeqCst));

        // LEAKED objects never reach 0
        if old.leaked() {
            return false;
        }

        old.strong() == 1
    }

    /// Mark this object as leaked (interned). It will never be deallocated.
    pub fn leak(&self) {
        debug_assert!(!self.is_leaked());
        let mut old = State::from_raw(self.state.load(Ordering::SeqCst));
        loop {
            let new_state = old.with_leaked(true);
            match self.state.compare_exchange(
                old.as_raw(),
                new_state.as_raw(),
                Ordering::SeqCst,
                Ordering::SeqCst,
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
}

// Deferred Drop Infrastructure
//
// This mechanism allows untrack_object() calls to be deferred until after
// the GC collection phase completes, preventing deadlocks that occur when
// clear (pop_edges) triggers object destruction while holding the tracked_objects lock.

thread_local! {
    /// Flag indicating if we're inside a deferred drop context.
    /// When true, drop operations should defer untrack calls.
    static IN_DEFERRED_CONTEXT: Cell<bool> = const { Cell::new(false) };

    /// Queue of deferred untrack operations.
    /// No Send bound needed - this is thread-local and only accessed from the same thread.
    static DEFERRED_QUEUE: RefCell<Vec<Box<dyn FnOnce()>>> = const { RefCell::new(Vec::new()) };
}

/// RAII guard for deferred drop context.
/// Restores the previous context state on drop, even if a panic occurs.
struct DeferredDropGuard {
    was_in_context: bool,
}

impl Drop for DeferredDropGuard {
    fn drop(&mut self) {
        IN_DEFERRED_CONTEXT.with(|in_ctx| {
            in_ctx.set(self.was_in_context);
        });
        // Only flush if we're the outermost context
        if !self.was_in_context {
            flush_deferred_drops();
        }
    }
}

/// Execute a function within a deferred drop context.
/// Any calls to `try_defer_drop` within this context will be queued
/// and executed when the context exits (even on panic).
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
///
/// Note: No `Send` bound - this is thread-local and runs on the same thread.
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
