//! A module containing [`lock_api`]-based lock types that are or are not `Send + Sync`
//! depending on whether the `threading` feature of this module is enabled.

use lock_api::{
    MappedMutexGuard, MappedRwLockReadGuard, MappedRwLockWriteGuard, Mutex, MutexGuard, RwLock,
    RwLockReadGuard, RwLockUpgradableReadGuard, RwLockWriteGuard,
};

cfg_if::cfg_if! {
    if #[cfg(feature = "threading")] {
        pub use parking_lot::{RawMutex, RawRwLock, RawThreadId};

        pub use std::sync::OnceLock as OnceCell;
        pub use core::cell::LazyCell;
    } else {
        mod cell_lock;
        pub use cell_lock::{RawCellMutex as RawMutex, RawCellRwLock as RawRwLock, SingleThreadId as RawThreadId};

        pub use core::cell::{LazyCell, OnceCell};
    }
}

// LazyLock: thread-safe lazy initialization for `static` items.
// In non-threading mode with std, use std::sync::LazyLock for safety
// (Rust test runner uses parallel threads even without the threading feature).
// Without std, use a LazyCell wrapper (truly single-threaded environments only).
cfg_if::cfg_if! {
    if #[cfg(any(feature = "threading", feature = "std"))] {
        pub use std::sync::LazyLock;
    } else {
        pub struct LazyLock<T, F = fn() -> T>(core::cell::LazyCell<T, F>);
        // SAFETY: This branch is only active when both "std" and "threading"
        // features are absent — i.e., truly single-threaded no_std environments
        // (e.g., embedded or bare-metal WASM). Without std, the Rust runtime
        // cannot spawn threads, so Sync is trivially satisfied.
        unsafe impl<T, F> Sync for LazyLock<T, F> {}

        impl<T, F: FnOnce() -> T> LazyLock<T, F> {
            pub const fn new(f: F) -> Self { Self(core::cell::LazyCell::new(f)) }
            pub fn force(this: &Self) -> &T { core::cell::LazyCell::force(&this.0) }
        }

        impl<T, F: FnOnce() -> T> core::ops::Deref for LazyLock<T, F> {
            type Target = T;
            fn deref(&self) -> &T { &self.0 }
        }
    }
}

mod immutable_mutex;
pub use immutable_mutex::*;
mod thread_mutex;
pub use thread_mutex::*;

pub type PyMutex<T> = Mutex<RawMutex, T>;
pub type PyMutexGuard<'a, T> = MutexGuard<'a, RawMutex, T>;
pub type PyMappedMutexGuard<'a, T> = MappedMutexGuard<'a, RawMutex, T>;
pub type PyImmutableMappedMutexGuard<'a, T> = ImmutableMappedMutexGuard<'a, RawMutex, T>;
pub type PyThreadMutex<T> = ThreadMutex<RawMutex, RawThreadId, T>;
pub type PyThreadMutexGuard<'a, T> = ThreadMutexGuard<'a, RawMutex, RawThreadId, T>;
pub type PyMappedThreadMutexGuard<'a, T> = MappedThreadMutexGuard<'a, RawMutex, RawThreadId, T>;

pub type PyRwLock<T> = RwLock<RawRwLock, T>;
pub type PyRwLockUpgradableReadGuard<'a, T> = RwLockUpgradableReadGuard<'a, RawRwLock, T>;
pub type PyRwLockReadGuard<'a, T> = RwLockReadGuard<'a, RawRwLock, T>;
pub type PyMappedRwLockReadGuard<'a, T> = MappedRwLockReadGuard<'a, RawRwLock, T>;
pub type PyRwLockWriteGuard<'a, T> = RwLockWriteGuard<'a, RawRwLock, T>;
pub type PyMappedRwLockWriteGuard<'a, T> = MappedRwLockWriteGuard<'a, RawRwLock, T>;

// can add fn const_{mutex,rw_lock}() if necessary, but we probably won't need to

/// Reset a `PyMutex` to its initial (unlocked) state after `fork()`.
///
/// After `fork()`, locks held by dead parent threads would deadlock in the
/// child. This writes `RawMutex::INIT` via the `Mutex::raw()` accessor,
/// bypassing the normal unlock path which may interact with parking_lot's
/// internal waiter queues.
///
/// # Safety
///
/// Must only be called from the single-threaded child process immediately
/// after `fork()`, before any other thread is created.
#[cfg(unix)]
pub unsafe fn reinit_mutex_after_fork<T: ?Sized>(mutex: &PyMutex<T>) {
    // Use Mutex::raw() to access the underlying lock without layout assumptions.
    // parking_lot::RawMutex (AtomicU8) and RawCellMutex (Cell<bool>) both
    // represent the unlocked state as all-zero bytes.
    unsafe {
        let raw = mutex.raw() as *const RawMutex as *mut u8;
        core::ptr::write_bytes(raw, 0, core::mem::size_of::<RawMutex>());
    }
}

/// Reset a `PyRwLock` to its initial (unlocked) state after `fork()`.
///
/// Same rationale as [`reinit_mutex_after_fork`] — dead threads' read or
/// write locks would cause permanent deadlock in the child.
///
/// # Safety
///
/// Must only be called from the single-threaded child process immediately
/// after `fork()`, before any other thread is created.
#[cfg(unix)]
pub unsafe fn reinit_rwlock_after_fork<T: ?Sized>(rwlock: &PyRwLock<T>) {
    unsafe {
        let raw = rwlock.raw() as *const RawRwLock as *mut u8;
        core::ptr::write_bytes(raw, 0, core::mem::size_of::<RawRwLock>());
    }
}

/// Reset a `PyThreadMutex` to its initial (unlocked, unowned) state after `fork()`.
///
/// `PyThreadMutex` is used by buffered IO objects (`BufferedReader`,
/// `BufferedWriter`, `TextIOWrapper`). If a dead parent thread held one of
/// these locks during `fork()`, the child would deadlock on any IO operation.
///
/// # Safety
///
/// Must only be called from the single-threaded child process immediately
/// after `fork()`, before any other thread is created.
#[cfg(unix)]
pub unsafe fn reinit_thread_mutex_after_fork<T: ?Sized>(mutex: &PyThreadMutex<T>) {
    unsafe { mutex.raw().reinit_after_fork() }
}
