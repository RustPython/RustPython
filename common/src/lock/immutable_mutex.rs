use std::fmt;
use std::marker::PhantomData;
use std::ops::Deref;

use lock_api::{MutexGuard, RawMutex};

/// A mutex guard that has an exclusive lock, but only an immutable reference; useful if you
/// need to map a mutex guard with a function that returns an `&T`. Construct using the
/// [`MapImmutable`] trait.
pub struct ImmutableMappedMutexGuard<'a, R: RawMutex, T: ?Sized> {
    raw: &'a R,
    data: *const T,
    _marker: PhantomData<(&'a T, R::GuardMarker)>,
}

// main constructor for ImmutableMappedMutexGuard
// TODO: patch lock_api to have a MappedMutexGuard::raw method, and have this implementation be for
// MappedMutexGuard
impl<'a, R: RawMutex, T: ?Sized> MapImmutable<'a, R, T> for MutexGuard<'a, R, T> {
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

impl<'a, R: RawMutex, T: ?Sized> ImmutableMappedMutexGuard<'a, R, T> {
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

impl<'a, R: RawMutex, T: ?Sized> Deref for ImmutableMappedMutexGuard<'a, R, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        // SAFETY: self.data is valid for the lifetime of the guard
        unsafe { &*self.data }
    }
}

impl<'a, R: RawMutex, T: ?Sized> Drop for ImmutableMappedMutexGuard<'a, R, T> {
    fn drop(&mut self) {
        // SAFETY: An ImmutableMappedMutexGuard always holds the lock
        unsafe { self.raw.unlock() }
    }
}

impl<'a, R: RawMutex, T: fmt::Debug + ?Sized> fmt::Debug for ImmutableMappedMutexGuard<'a, R, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

impl<'a, R: RawMutex, T: fmt::Display + ?Sized> fmt::Display
    for ImmutableMappedMutexGuard<'a, R, T>
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&**self, f)
    }
}

pub trait MapImmutable<'a, R: RawMutex, T: ?Sized> {
    fn map_immutable<U: ?Sized, F>(s: Self, f: F) -> ImmutableMappedMutexGuard<'a, R, U>
    where
        F: FnOnce(&T) -> &U;
}
