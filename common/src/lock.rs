//! A module containing [`lock_api`]-based lock types that are or are not `Send + Sync`
//! depending on whether the `threading` feature of this module is enabled.

use lock_api::{
    MappedMutexGuard, MappedRwLockReadGuard, MappedRwLockWriteGuard, Mutex, MutexGuard, RwLock,
    RwLockReadGuard, RwLockUpgradableReadGuard, RwLockWriteGuard,
};

cfg_if::cfg_if! {
    if #[cfg(feature = "threading")] {
        pub use parking_lot::{RawMutex, RawRwLock, RawThreadId};

        pub use once_cell::sync::{Lazy, OnceCell};
    } else {
        mod cell_lock;
        pub use cell_lock::{RawCellMutex as RawMutex, RawCellRwLock as RawRwLock, SingleThreadId as RawThreadId};

        pub use once_cell::unsync::{Lazy, OnceCell};
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

// can add fn const_{mutex,rwlock}() if necessary, but we probably won't need to
