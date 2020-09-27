//! A module containing [`lock_api`]-based lock types that are or are not `Send + Sync`
//! depending on whether the `threading` feature of this module is enabled.

use std::fmt;
use std::marker::PhantomData;
use std::ops::Deref;

use lock_api::{
    MappedMutexGuard, MappedRwLockReadGuard, MappedRwLockWriteGuard, Mutex, MutexGuard,
    RawMutex as RawMutex_, RwLock, RwLockReadGuard, RwLockUpgradableReadGuard, RwLockWriteGuard,
};

cfg_if::cfg_if! {
    if #[cfg(feature = "threading")] {
        pub use parking_lot::{RawMutex, RawRwLock};

        pub use once_cell::sync::{Lazy, OnceCell};
    } else {
        mod cell_lock;
        pub use cell_lock::{RawCellMutex as RawMutex, RawCellRwLock as RawRwLock};

        pub use once_cell::unsync::{Lazy, OnceCell};
    }
}

pub type PyMutex<T> = Mutex<RawMutex, T>;
pub type PyMutexGuard<'a, T> = MutexGuard<'a, RawMutex, T>;
pub type PyMappedMutexGuard<'a, T> = MappedMutexGuard<'a, RawMutex, T>;
pub type PyImmutableMappedMutexGuard<'a, T> = ImmutableMappedMutexGuard<'a, RawMutex, T>;

pub type PyRwLock<T> = RwLock<RawRwLock, T>;
pub type PyRwLockUpgradableReadGuard<'a, T> = RwLockUpgradableReadGuard<'a, RawRwLock, T>;
pub type PyRwLockReadGuard<'a, T> = RwLockReadGuard<'a, RawRwLock, T>;
pub type PyMappedRwLockReadGuard<'a, T> = MappedRwLockReadGuard<'a, RawRwLock, T>;
pub type PyRwLockWriteGuard<'a, T> = RwLockWriteGuard<'a, RawRwLock, T>;
pub type PyMappedRwLockWriteGuard<'a, T> = MappedRwLockWriteGuard<'a, RawRwLock, T>;

// can add fn const_{mutex,rwlock}() if necessary, but we probably won't need to

/// A mutex guard that has an exclusive lock, but only an immutable reference; useful if you
/// need to map a mutex guard with a function that returns an `&T`. Construct using the
/// [`MapImmutable`] trait.
pub struct ImmutableMappedMutexGuard<'a, R: RawMutex_, T: ?Sized> {
    raw: &'a R,
    data: *const T,
    _marker: PhantomData<(&'a T, <RawMutex as RawMutex_>::GuardMarker)>,
}

// main constructor for ImmutableMappedMutexGuard
// TODO: patch lock_api to have a MappedMutexGuard::raw method, and have this implementation be for
// MappedMutexGuard
impl<'a, R: RawMutex_, T: ?Sized> MapImmutable<'a, R, T> for MutexGuard<'a, R, T> {
    fn map_immutable<U: ?Sized, F>(s: Self, f: F) -> ImmutableMappedMutexGuard<'a, R, U>
    where
        F: FnOnce(&T) -> &U,
    {
        let raw = unsafe { MutexGuard::mutex(&s).raw() };
        let data = f(&s) as *const U;
        std::mem::forget(s);
        ImmutableMappedMutexGuard {
            raw,
            data,
            _marker: PhantomData,
        }
    }
}

impl<'a, R: RawMutex_, T: ?Sized> ImmutableMappedMutexGuard<'a, R, T> {
    pub fn map<U: ?Sized, F>(s: Self, f: F) -> ImmutableMappedMutexGuard<'a, R, U>
    where
        F: FnOnce(&T) -> &U,
    {
        let raw = s.raw;
        let data = f(&s) as *const U;
        std::mem::forget(s);
        ImmutableMappedMutexGuard {
            raw,
            data,
            _marker: PhantomData,
        }
    }
}

impl<'a, R: RawMutex_, T: ?Sized> Deref for ImmutableMappedMutexGuard<'a, R, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        // SAFETY: self.data is valid for the lifetime of the guard
        unsafe { &*self.data }
    }
}

impl<'a, R: RawMutex_, T: ?Sized> Drop for ImmutableMappedMutexGuard<'a, R, T> {
    fn drop(&mut self) {
        // SAFETY: An ImmutableMappedMutexGuard always holds the lock
        unsafe { self.raw.unlock() }
    }
}

impl<'a, R: RawMutex_, T: fmt::Debug + ?Sized> fmt::Debug for ImmutableMappedMutexGuard<'a, R, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

impl<'a, R: RawMutex_, T: fmt::Display + ?Sized> fmt::Display
    for ImmutableMappedMutexGuard<'a, R, T>
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&**self, f)
    }
}

pub trait MapImmutable<'a, R: RawMutex_, T: ?Sized> {
    fn map_immutable<U: ?Sized, F>(s: Self, f: F) -> ImmutableMappedMutexGuard<'a, R, U>
    where
        F: FnOnce(&T) -> &U;
}
