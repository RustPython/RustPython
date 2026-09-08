use crate::atomic::{Ordering, PyAtomic, Radium};

// State layout (usize):
//   [1 bit: destructed] [1 bit: published] [1 bit: leaked] [1 bit: immortal]
//   [M bits: strong_count]
// 64-bit: M=60.  32-bit: M=28.
//
// Weak references live in the object's `WeakRefList`, not in this word, so the
// strong count takes every bit the flags leave. A 32-bit target reaches its
// ceiling at 268 435 455 references rather than the 32 767 that half the word
// would allow — a number two ordinary module imports pass on `wasm32`.
const FLAG_BITS: u32 = 4;
const DESTRUCTED: usize = 1 << (usize::BITS - 1);
/// Object was published to a lock-free cache; memory reclamation is
/// deferred through QSBR so concurrent try-incref readers never touch
/// freed memory. Sticky once set.
const PUBLISHED: usize = 1 << (usize::BITS - 2);
const LEAKED: usize = 1 << (usize::BITS - 3);
/// Object lives for the whole process (PEP 683). Sticky once set.
///
/// # The immortal invariant
///
/// Once [`RefCount::make_immortal`] has run, the word never changes again:
/// `inc`, `inc_by`, `dec` and `safe_inc` read the bit and return without
/// touching the counter, so every reference operation on such an object is a
/// relaxed load and a well-predicted branch rather than an atomic
/// read-modify-write — and `dec` in particular drops its `Release` store,
/// which is the expensive half of a refcount pair on a weakly ordered target.
///
/// The consequences the rest of the tree may rely on:
///
/// * `dec` never reports the object collectable, so it is never deallocated
///   and no `__del__` or weakref callback ever fires for it. Immortality may
///   therefore only be granted to something an owner keeps for the whole
///   process (a `static_cell`, the interpreter's `Context`).
/// * `get` reports [`IMMORTAL_COUNT`], a number far past any real reference
///   total. Unique-ownership fast paths spelled `strong_count() == 1` can
///   therefore never fire on an immortal object, and the cycle collector's
///   `start_gc_refs` clamps a count that large straight to `GC_REACHABLE`, so
///   an immortal object is a permanent root that keeps its referents alive.
/// * It is not the same answer as [`LEAKED`], which means "the string pool
///   owns this copy" and is what `PyObject::is_interned` — and the dict
///   pointer-equality key fast path behind it — reads. Interning implies
///   immortality ([`RefCount::leak`] sets both, which is what lets `dec`
///   decide "a leaked object never reaches zero" from the immortal bit
///   alone), but an immortal object is not thereby interned.
const IMMORTAL: usize = 1 << (usize::BITS - 4);
const STRONG_WIDTH: u32 = usize::BITS - FLAG_BITS;
const STRONG: usize = (1 << STRONG_WIDTH) - 1;
const COUNT: usize = 1;

/// The strong count an immortal object's word is parked at.
///
/// On a 64-bit target this is CPython's `_Py_IMMORTAL_REFCNT`, `UINT_MAX`, so
/// `sys.getrefcount(None)` reports the same 4294967295 CPython does — and it
/// is exactly `GC_REACHABLE`, which is what makes the collector treat an
/// immortal candidate as reachable without a special case. A 32-bit strong
/// field cannot hold that, so there it takes half the field instead: still
/// more references than a subtraction pass could ever walk down.
const IMMORTAL_COUNT: usize = if (u32::MAX as u64) < STRONG as u64 {
    u32::MAX as usize
} else {
    STRONG / 2
};

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
    const fn immortal(self) -> bool {
        (self.inner & IMMORTAL) != 0
    }

    #[inline]
    fn add_strong(self, val: u32) -> Self {
        Self::from_raw(self.inner + (val as usize) * COUNT)
    }

    #[inline]
    fn with_leaked(self, leaked: bool) -> Self {
        Self::from_raw((self.inner & !LEAKED) | if leaked { LEAKED } else { 0 })
    }

    /// The same flags, with [`IMMORTAL`] set and the strong count parked at
    /// [`IMMORTAL_COUNT`].
    #[inline]
    fn immortalized(self) -> Self {
        Self::from_raw((self.inner & !STRONG) | IMMORTAL | (IMMORTAL_COUNT * COUNT))
    }
}

/// Reference count using state layout with LEAKED and IMMORTAL support.
///
/// State layout (usize):
/// 64-bit: [destructed] [published] [leaked] [immortal] [60 bits: strong_count]
/// 32-bit: [destructed] [published] [leaked] [immortal] [28 bits: strong_count]
///
/// See [`IMMORTAL`] for what the immortal bit promises.
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

    /// Whether this object lives for the whole process.
    ///
    /// A plain relaxed load: the bit is sticky and is set before the object is
    /// shared, so no reader can observe it flipping.
    #[inline(always)]
    #[must_use]
    pub fn is_immortal(&self) -> bool {
        State::from_raw(self.state.load(Ordering::Relaxed)).immortal()
    }

    /// Increment strong count
    #[inline(always)]
    pub fn inc(&self) {
        if self.is_immortal() {
            return;
        }
        let val = State::from_raw(self.state.fetch_add(COUNT, Ordering::Relaxed));
        // One comparison stands in for the three cases that are not an
        // ordinary increment. Masking to the count and the destructed bit and
        // subtracting one leaves an ordinary count below `STRONG - COUNT`: a
        // count of zero wraps above it, a count at the ceiling lands on it,
        // and a destructed word carries a bit far above the count field. The
        // three of them written out separately cost the hot path two extra
        // compares, which is what the immortal test above wants back.
        if (val.as_raw() & (DESTRUCTED | STRONG)).wrapping_sub(COUNT) >= STRONG - COUNT {
            self.inc_uncommon(val);
        }
    }

    /// The `inc` cases that are not an ordinary increment: an overflowed or
    /// destructed word, and a count of zero — where the `fetch_add` that just
    /// ran created a permission to run the decrement again.
    #[cold]
    #[inline(never)]
    fn inc_uncommon(&self, val: State) {
        if val.destructed() || val.strong() > STRONG - 1 {
            refcount_overflow();
        }
        self.state.fetch_add(COUNT, Ordering::Relaxed);
    }

    #[inline(always)]
    pub fn inc_by(&self, n: usize) {
        debug_assert!(n <= STRONG);
        if self.is_immortal() {
            return;
        }
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
            if old.immortal() {
                return true;
            }
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
    #[inline(always)]
    #[must_use]
    pub fn dec(&self) -> bool {
        // The whole point of the immortal bit: this returns before the
        // `Release` read-modify-write, which is the costly half of a refcount
        // pair on a weakly ordered target.
        //
        // The test also stands in for the one this used to make on the result
        // of the decrement — "LEAKED objects never reach 0". `leak` sets
        // `IMMORTAL` alongside `LEAKED`, so an interned object leaves here on
        // the line above and the counter it would have walked down is never
        // touched. That is what keeps the guard from costing an instruction
        // on the mortal path: two go in at the top, one comes out below.
        if self.is_immortal() {
            return false;
        }
        let old = State::from_raw(self.state.fetch_sub(COUNT, Ordering::Release));
        debug_assert!(!old.leaked(), "a leaked object must also be immortal");

        if old.strong() == 1 {
            core::sync::atomic::fence(Ordering::Acquire);
            return true;
        }
        false
    }

    /// Mark this object as leaked (interned). It will never be deallocated.
    ///
    /// This also makes the object immortal, and [`RefCount::dec`] depends on
    /// that: leaked and immortal are separate answers — `is_leaked` means "the
    /// string pool owns this copy" and is what pointer-equality key lookups
    /// read — but every leaked object is immortal, so `dec` can decide both
    /// from the immortal bit alone.
    pub fn leak(&self) {
        debug_assert!(!self.is_leaked());
        let mut old = State::from_raw(self.state.load(Ordering::Relaxed));
        loop {
            let new_state = old.with_leaked(true).immortalized();
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

    /// Make this object immortal: every later `inc`/`dec` becomes a branch and
    /// the object is never deallocated. See [`IMMORTAL`] for the invariant.
    ///
    /// Idempotent, and safe to call on an object that is already interned —
    /// the other flags are preserved.
    pub fn make_immortal(&self) {
        let mut old = State::from_raw(self.state.load(Ordering::Relaxed));
        loop {
            if old.immortal() {
                return;
            }
            debug_assert!(!old.destructed() && old.strong() > 0);
            match self.state.compare_exchange_weak(
                old.as_raw(),
                old.immortalized().as_raw(),
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

    /// `inc` and `dec` reach the same ceiling as `inc_by`.
    ///
    /// The aborts reported against this layout came one reference at a time
    /// through `inc`, whose overflow check is written separately from
    /// `inc_by`'s, and the count has to come back down through `dec` without
    /// reporting the object collectable before the last reference goes.
    #[test]
    fn inc_and_dec_reach_past_a_16_bit_ceiling() {
        const REFERENCES: usize = 1 << 20;

        let rc = RefCount::new(); // strong = 1
        for _ in 1..REFERENCES {
            rc.inc();
        }
        assert_eq!(rc.get(), REFERENCES);
        for _ in 1..REFERENCES {
            assert!(!rc.dec());
        }
        assert_eq!(rc.get(), 1);
        assert!(rc.dec());
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

    /// An immortal count neither moves nor ever reports the object collectable.
    ///
    /// This is the whole contract the singletons rely on: `inc`/`dec` become
    /// branches, so a `dec` that would have been the last one still answers
    /// "not collectable", and `sys.getrefcount` keeps reporting the same
    /// number however much reference traffic passes through.
    #[test]
    fn an_immortal_count_never_moves() {
        let rc = RefCount::new(); // strong = 1
        assert!(!rc.is_immortal());
        rc.make_immortal();
        assert!(rc.is_immortal());
        assert_eq!(rc.get(), IMMORTAL_COUNT);

        rc.inc();
        rc.inc_by(1000);
        assert!(rc.safe_inc());
        assert_eq!(rc.get(), IMMORTAL_COUNT);

        for _ in 0..1000 {
            assert!(!rc.dec());
        }
        assert_eq!(rc.get(), IMMORTAL_COUNT);

        // Idempotent.
        rc.make_immortal();
        assert_eq!(rc.get(), IMMORTAL_COUNT);
    }

    /// Immortality does not imply interning, and interning does imply
    /// immortality.
    ///
    /// `PyObject::is_interned` answers from `LEAKED`, and the dict key fast
    /// path turns that answer into pointer equality, so immortalizing an
    /// object must not make it look interned. The other direction is the
    /// implication `dec` leans on when it decides "leaked objects never reach
    /// zero" from the immortal bit alone.
    #[test]
    fn interning_implies_immortality_but_not_the_other_way() {
        let immortal = RefCount::new();
        immortal.make_immortal();
        assert!(immortal.is_immortal());
        assert!(!immortal.is_leaked());

        let interned = RefCount::new();
        interned.leak();
        assert!(interned.is_leaked());
        assert!(interned.is_immortal());
        assert_eq!(interned.get(), IMMORTAL_COUNT);

        interned.inc();
        assert!(!interned.dec());
        assert!(!interned.dec());
        assert_eq!(interned.get(), IMMORTAL_COUNT);
    }

    /// The parked count is large enough for the collector's reachable clamp.
    ///
    /// `PyObject::start_gc_refs` treats a strong count of `u32::MAX` or more
    /// as reachable outright; on a 64-bit host that clamp is the only thing
    /// keeping an immortal object out of a collection's dead set.
    // Both operands are constants, so these hold at compile time or not at
    // all; a runtime `assert!` would only be dead weight (and `clippy` says so).
    const _: () = assert!(IMMORTAL_COUNT > 1);
    const _: () = assert!(IMMORTAL_COUNT <= STRONG);
    const _: () = assert!(usize::BITS < 64 || IMMORTAL_COUNT == u32::MAX as usize);

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
