//! A reader-writer lock that lets a thread leave its interpreter before it
//! blocks.
//!
//! Stopping the world means waiting for every running thread to reach a
//! safepoint. A thread blocked on a lock reaches none, so if the thread holding
//! that lock has already been stopped, the two wait on each other forever. The
//! holder is not the one who can avoid this — a lock is held across a blocking
//! call precisely because that is what the call needs — so the waiter gives up
//! its interpreter for the duration of the wait instead, which is what a
//! blocking call does anyway.
//!
//! Doing so is safe only for locks nothing reachable from a stop-the-world
//! section takes, so it is opt-in per lock — see [`RawDetachingRwLock`] for the
//! rule and why it is needed.
//!
//! Only the contended path pays for any of this: an acquire that takes the lock
//! on the first try is the same atomic exchange it was, and never reaches the
//! hook. The hook is installed by whoever knows how to detach a thread
//! ([`set_blocking_wait_hook`]); until then, and on any thread that is not
//! running an interpreter, a blocked acquire just blocks.

use super::RawRwLock;
#[cfg(feature = "threading")]
use core::cell::Cell;
use lock_api::{
    RawRwLock as RawRwLockTrait, RawRwLockDowngrade, RawRwLockRecursive as RawRwLockRecursiveTrait,
    RawRwLockUpgrade as RawRwLockUpgradeTrait, RawRwLockUpgradeDowngrade,
};
#[cfg(feature = "threading")]
use std::sync::OnceLock;

/// Runs `wait` with the calling thread detached from its interpreter.
#[cfg(feature = "threading")]
pub type BlockingWaitHook = fn(wait: &dyn Fn());

#[cfg(feature = "threading")]
static BLOCKING_WAIT: OnceLock<BlockingWaitHook> = OnceLock::new();

/// Install the hook that detaches a thread around a blocked lock acquire.
///
/// Later calls are ignored, so every interpreter in a process can call this
/// during its own initialization.
#[cfg(feature = "threading")]
pub fn set_blocking_wait_hook(hook: BlockingWaitHook) {
    let _ = BLOCKING_WAIT.set(hook);
}

#[cfg(feature = "threading")]
std::thread_local! {
    /// Set while this thread is inside the hook, so that a lock taken by the
    /// hook itself — or by anything detaching and re-attaching runs — waits
    /// plainly instead of recursing back into it.
    static IN_HOOK: Cell<bool> = const { Cell::new(false) };
}

/// Clears [`IN_HOOK`] even if the hook unwinds.
#[cfg(feature = "threading")]
struct HookGuard;

#[cfg(feature = "threading")]
impl Drop for HookGuard {
    fn drop(&mut self) {
        let _ = IN_HOOK.try_with(|in_hook| in_hook.set(false));
    }
}

#[cfg(all(feature = "threading", debug_assertions))]
std::thread_local! {
    /// Set on the one thread still running while the world is stopped.
    static WORLD_STOPPED: Cell<bool> = const { Cell::new(false) };
}

/// Record whether this thread is the one running inside a stopped world.
///
/// The rule for opting a lock into detaching is that nothing reachable from a
/// stop-the-world section takes it — a section that did could block on a lock
/// only that same section can release. Not implementing `Traverse` states the
/// rule to a collection; this states it to every other section, which is
/// otherwise unchecked. Debug builds only; release builds track nothing and
/// pay nothing.
#[cfg(feature = "threading")]
#[inline]
pub fn set_world_stopped(stopped: bool) {
    #[cfg(debug_assertions)]
    let _ = WORLD_STOPPED.try_with(|flag| flag.set(stopped));
    #[cfg(not(debug_assertions))]
    let _ = stopped;
}

/// Panics if a stop-the-world section is taking one of these locks.
#[cfg(all(feature = "threading", debug_assertions))]
#[track_caller]
fn assert_not_stopping_the_world() {
    // `try_with` fails only once thread locals are being destroyed, which is
    // not a point at which this thread is driving a stop.
    let stopped = WORLD_STOPPED.try_with(Cell::get).unwrap_or(false);
    assert!(
        !stopped,
        "a stop-the-world section took a detaching lock, which a parked thread \
         may be holding and only this section can release"
    );
}

#[cfg(not(all(feature = "threading", debug_assertions)))]
#[inline(always)]
fn assert_not_stopping_the_world() {}

/// Block on `wait`, detached from this thread's interpreter if there is one.
///
/// Nothing spins on the way here. The lock underneath already spins before it
/// parks, and skips that spin once a waiter has parked — the same condition
/// `_PyMutex_LockTimed` spins under. A spin layered on top cannot read that
/// condition, and would go on retrying a `try_lock` that reports failure for as
/// long as a writer holds the writer bit, which it takes before it waits for
/// readers to drain: a yield per retry for the whole of exactly the wait this
/// exists to survive.
#[cfg(feature = "threading")]
#[cold]
#[inline(never)]
fn wait_detached(wait: impl Fn()) {
    let Some(hook) = BLOCKING_WAIT.get() else {
        wait();
        return;
    };
    // `try_with` fails once the thread's locals are being destroyed, which is
    // also a point at which there is no interpreter left to detach from.
    let entered = IN_HOOK
        .try_with(|in_hook| !in_hook.replace(true))
        .unwrap_or(false);
    if !entered {
        wait();
        return;
    }
    let _guard = HookGuard;
    hook(&wait);
}

/// Without threads there is no interpreter to leave and nothing to stop.
#[cfg(not(feature = "threading"))]
#[inline]
fn wait_detached(wait: impl Fn()) {
    wait();
}

/// A reader-writer lock whose blocking acquires detach first, and which is the
/// raw lock it wraps in every other respect.
///
/// Use through [`PyDetachingRwLock`](super::PyDetachingRwLock).
///
/// # Only for locks a collection never takes
///
/// The wait acquires the lock while detached, so the thread comes back holding
/// it, and re-attaching is a point at which a stop-the-world in flight will
/// park the thread. It is therefore parked *holding the lock*. Everything that
/// stops the world must be able to finish without that lock: if a collection
/// were to take it, the collection would block on a thread only the collection
/// can release, and neither would move again.
///
/// So this is opt-in per lock, and the rule for opting in is that nothing
/// reachable from a stop-the-world section takes the same lock. An object whose
/// payload holds no references — nothing for the collector to traverse into —
/// satisfies that; most do not.
///
/// Not implementing the vm's `Traverse` for this lock enforces part of that: a
/// payload holding one cannot derive `Traverse`, so it cannot become something
/// a collection walks into. Only that part. A collection is not the only thing
/// that stops the world — dumping tracebacks, enumerating thread frames and
/// forking all do — and nothing checks what those reach. For them the rule is
/// still a convention.
#[repr(transparent)]
pub struct RawDetachingRwLock(RawRwLock);

// SAFETY: every method forwards to the wrapped raw lock, which upholds the
// contract; the blocking acquires only add a wait that ends with the same lock
// acquired.
unsafe impl RawRwLockTrait for RawDetachingRwLock {
    #[allow(
        clippy::declare_interior_mutable_const,
        reason = "raw lock initializer, as in the type it wraps"
    )]
    const INIT: Self = Self(<RawRwLock as RawRwLockTrait>::INIT);

    type GuardMarker = <RawRwLock as RawRwLockTrait>::GuardMarker;

    #[inline]
    fn lock_shared(&self) {
        assert_not_stopping_the_world();
        if !self.0.try_lock_shared() {
            wait_detached(|| self.0.lock_shared());
        }
    }

    #[inline]
    fn try_lock_shared(&self) -> bool {
        self.0.try_lock_shared()
    }

    #[inline]
    unsafe fn unlock_shared(&self) {
        unsafe { self.0.unlock_shared() }
    }

    #[inline]
    fn lock_exclusive(&self) {
        assert_not_stopping_the_world();
        if !self.0.try_lock_exclusive() {
            wait_detached(|| self.0.lock_exclusive());
        }
    }

    #[inline]
    fn try_lock_exclusive(&self) -> bool {
        self.0.try_lock_exclusive()
    }

    #[inline]
    unsafe fn unlock_exclusive(&self) {
        unsafe { self.0.unlock_exclusive() }
    }

    #[inline]
    fn is_locked(&self) -> bool {
        self.0.is_locked()
    }

    #[inline]
    fn is_locked_exclusive(&self) -> bool {
        self.0.is_locked_exclusive()
    }
}

// SAFETY: forwards to the wrapped raw lock.
unsafe impl RawRwLockDowngrade for RawDetachingRwLock {
    #[inline]
    unsafe fn downgrade(&self) {
        unsafe { self.0.downgrade() }
    }
}

// SAFETY: forwards to the wrapped raw lock.
//
// None of these detach. `upgrade` runs with the upgradable lock already held,
// and `lock_shared_recursive` may be the re-entrant take of a lock this thread
// holds; detaching there would park a thread *holding* the lock, the one thing
// this type must not do. `lock_upgradable` starts from holding nothing and
// could detach as safely as `lock_shared` does, but nothing takes an upgradable
// read of one of these, so it does not.
unsafe impl RawRwLockUpgradeTrait for RawDetachingRwLock {
    #[inline]
    fn lock_upgradable(&self) {
        self.0.lock_upgradable()
    }

    #[inline]
    fn try_lock_upgradable(&self) -> bool {
        self.0.try_lock_upgradable()
    }

    #[inline]
    unsafe fn unlock_upgradable(&self) {
        unsafe { self.0.unlock_upgradable() }
    }

    #[inline]
    unsafe fn upgrade(&self) {
        // SAFETY: the caller holds the upgradable lock, as `upgrade` requires.
        unsafe { self.0.upgrade() }
    }

    #[inline]
    unsafe fn try_upgrade(&self) -> bool {
        unsafe { self.0.try_upgrade() }
    }
}

// SAFETY: forwards to the wrapped raw lock.
unsafe impl RawRwLockUpgradeDowngrade for RawDetachingRwLock {
    #[inline]
    unsafe fn downgrade_upgradable(&self) {
        unsafe { self.0.downgrade_upgradable() }
    }

    #[inline]
    unsafe fn downgrade_to_upgradable(&self) {
        unsafe { self.0.downgrade_to_upgradable() }
    }
}

// SAFETY: forwards to the wrapped raw lock. Does not detach; see the upgrade
// impl above.
unsafe impl RawRwLockRecursiveTrait for RawDetachingRwLock {
    #[inline]
    fn lock_shared_recursive(&self) {
        self.0.lock_shared_recursive()
    }

    #[inline]
    fn try_lock_shared_recursive(&self) -> bool {
        self.0.try_lock_shared_recursive()
    }
}

#[cfg(test)]
mod tests {
    #[cfg(all(feature = "threading", debug_assertions))]
    use super::set_world_stopped;
    #[cfg(all(feature = "threading", debug_assertions))]
    use crate::lock::PyDetachingRwLock;

    /// The opt-in rule holds for every stop-the-world section, not only the
    /// collector that not implementing `Traverse` speaks to.
    #[cfg(all(feature = "threading", debug_assertions))]
    #[test]
    fn taking_one_while_stopping_the_world_is_caught() {
        let lock = PyDetachingRwLock::new(());

        // Ordinary use, for contrast.
        drop(lock.write());

        set_world_stopped(true);
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let taken = std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| {
            let _guard = lock.read();
        }));
        std::panic::set_hook(hook);
        set_world_stopped(false);

        assert!(
            taken.is_err(),
            "a stop-the-world section took a detaching lock and nothing complained"
        );

        // The flag is per-thread and back to clear, so the lock still works.
        drop(lock.write());
    }
}
