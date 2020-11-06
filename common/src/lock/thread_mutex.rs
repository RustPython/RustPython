use std::cell::UnsafeCell;
use std::fmt;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicUsize, Ordering};

use lock_api::{GetThreadId, GuardNoSend, RawMutex};

// based off ReentrantMutex from lock_api

/// A mutex type that knows when it would deadlock
pub struct RawThreadMutex<R: RawMutex, G: GetThreadId> {
    owner: AtomicUsize,
    mutex: R,
    get_thread_id: G,
}

impl<R: RawMutex, G: GetThreadId> RawThreadMutex<R, G> {
    #[allow(clippy::declare_interior_mutable_const)]
    pub const INIT: Self = RawThreadMutex {
        owner: AtomicUsize::new(0),
        mutex: R::INIT,
        get_thread_id: G::INIT,
    };

    #[inline]
    fn lock_internal<F: FnOnce() -> bool>(&self, try_lock: F) -> Option<bool> {
        let id = self.get_thread_id.nonzero_thread_id().get();
        if self.owner.load(Ordering::Relaxed) == id {
            return None;
        } else {
            if !try_lock() {
                return Some(false);
            }
            self.owner.store(id, Ordering::Relaxed);
        }
        Some(true)
    }

    /// Blocks for the mutex to be available, and returns true if the mutex isn't already
    /// locked on the current thread.
    pub fn lock(&self) -> bool {
        self.lock_internal(|| {
            self.mutex.lock();
            true
        })
        .is_some()
    }

    /// Returns `Some(true)` if able to successfully lock without blocking, `Some(false)`
    /// otherwise, and `None` when the mutex is already locked on the current thread.
    pub fn try_lock(&self) -> Option<bool> {
        self.lock_internal(|| self.mutex.try_lock())
    }

    /// Unlocks this mutex. The inner mutex may not be unlocked if
    /// this mutex was acquired previously in the current thread.
    ///
    /// # Safety
    ///
    /// This method may only be called if the mutex is held by the current thread.
    pub unsafe fn unlock(&self) {
        self.owner.store(0, Ordering::Relaxed);
        self.mutex.unlock();
    }
}

unsafe impl<R: RawMutex + Send, G: GetThreadId + Send> Send for RawThreadMutex<R, G> {}
unsafe impl<R: RawMutex + Sync, G: GetThreadId + Sync> Sync for RawThreadMutex<R, G> {}

pub struct ThreadMutex<R: RawMutex, G: GetThreadId, T: ?Sized> {
    raw: RawThreadMutex<R, G>,
    data: UnsafeCell<T>,
}

impl<R: RawMutex, G: GetThreadId, T> ThreadMutex<R, G, T> {
    pub fn new(val: T) -> Self {
        ThreadMutex {
            raw: RawThreadMutex::INIT,
            data: UnsafeCell::new(val),
        }
    }

    pub fn into_inner(self) -> T {
        self.data.into_inner()
    }
}
impl<R: RawMutex, G: GetThreadId, T: Default> Default for ThreadMutex<R, G, T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}
impl<R: RawMutex, G: GetThreadId, T: ?Sized> ThreadMutex<R, G, T> {
    pub fn lock(&self) -> Option<ThreadMutexGuard<R, G, T>> {
        if self.raw.lock() {
            Some(ThreadMutexGuard {
                mu: self,
                marker: PhantomData,
            })
        } else {
            None
        }
    }
    pub fn try_lock(&self) -> Result<ThreadMutexGuard<R, G, T>, TryLockThreadError> {
        match self.raw.try_lock() {
            Some(true) => Ok(ThreadMutexGuard {
                mu: self,
                marker: PhantomData,
            }),
            Some(false) => Err(TryLockThreadError::Other),
            None => Err(TryLockThreadError::Current),
        }
    }
}
// Whether ThreadMutex::try_lock failed because the mutex was already locked on another thread or
// on the current thread
pub enum TryLockThreadError {
    Other,
    Current,
}

struct LockedPlaceholder(&'static str);
impl fmt::Debug for LockedPlaceholder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl<R: RawMutex, G: GetThreadId, T: ?Sized + fmt::Debug> fmt::Debug for ThreadMutex<R, G, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.try_lock() {
            Ok(guard) => f
                .debug_struct("ThreadMutex")
                .field("data", &&*guard)
                .finish(),
            Err(e) => {
                let msg = match e {
                    TryLockThreadError::Other => "<locked on other thread>",
                    TryLockThreadError::Current => "<locked on current thread>",
                };
                f.debug_struct("ThreadMutex")
                    .field("data", &LockedPlaceholder(msg))
                    .finish()
            }
        }
    }
}

unsafe impl<R: RawMutex + Send, G: GetThreadId + Send, T: ?Sized + Send> Send
    for ThreadMutex<R, G, T>
{
}
unsafe impl<R: RawMutex + Sync, G: GetThreadId + Sync, T: ?Sized + Send> Sync
    for ThreadMutex<R, G, T>
{
}

pub struct ThreadMutexGuard<'a, R: RawMutex, G: GetThreadId, T: ?Sized> {
    mu: &'a ThreadMutex<R, G, T>,
    marker: PhantomData<(&'a mut T, GuardNoSend)>,
}
impl<'a, R: RawMutex, G: GetThreadId, T: ?Sized> ThreadMutexGuard<'a, R, G, T> {
    pub fn map<U, F: FnOnce(&mut T) -> &mut U>(
        mut s: Self,
        f: F,
    ) -> MappedThreadMutexGuard<'a, R, G, U> {
        let data = f(&mut s).into();
        let mu = &s.mu.raw;
        std::mem::forget(s);
        MappedThreadMutexGuard {
            mu,
            data,
            marker: PhantomData,
        }
    }
    pub fn try_map<U, F: FnOnce(&mut T) -> Option<&mut U>>(
        mut s: Self,
        f: F,
    ) -> Result<MappedThreadMutexGuard<'a, R, G, U>, Self> {
        if let Some(data) = f(&mut s) {
            let data = data.into();
            let mu = &s.mu.raw;
            std::mem::forget(s);
            Ok(MappedThreadMutexGuard {
                mu,
                data,
                marker: PhantomData,
            })
        } else {
            Err(s)
        }
    }
}
impl<'a, R: RawMutex, G: GetThreadId, T: ?Sized> Deref for ThreadMutexGuard<'a, R, G, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.mu.data.get() }
    }
}
impl<'a, R: RawMutex, G: GetThreadId, T: ?Sized> DerefMut for ThreadMutexGuard<'a, R, G, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.mu.data.get() }
    }
}
impl<'a, R: RawMutex, G: GetThreadId, T: ?Sized> Drop for ThreadMutexGuard<'a, R, G, T> {
    fn drop(&mut self) {
        unsafe { self.mu.raw.unlock() }
    }
}
impl<'a, R: RawMutex, G: GetThreadId, T: ?Sized + fmt::Display> fmt::Display
    for ThreadMutexGuard<'a, R, G, T>
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(&**self, f)
    }
}
impl<'a, R: RawMutex, G: GetThreadId, T: ?Sized + fmt::Debug> fmt::Debug
    for ThreadMutexGuard<'a, R, G, T>
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}
pub struct MappedThreadMutexGuard<'a, R: RawMutex, G: GetThreadId, T: ?Sized> {
    mu: &'a RawThreadMutex<R, G>,
    data: NonNull<T>,
    marker: PhantomData<(&'a mut T, GuardNoSend)>,
}
impl<'a, R: RawMutex, G: GetThreadId, T: ?Sized> MappedThreadMutexGuard<'a, R, G, T> {
    pub fn map<U, F: FnOnce(&mut T) -> &mut U>(
        mut s: Self,
        f: F,
    ) -> MappedThreadMutexGuard<'a, R, G, U> {
        let data = f(&mut s).into();
        let mu = s.mu;
        std::mem::forget(s);
        MappedThreadMutexGuard {
            mu,
            data,
            marker: PhantomData,
        }
    }
    pub fn try_map<U, F: FnOnce(&mut T) -> Option<&mut U>>(
        mut s: Self,
        f: F,
    ) -> Result<MappedThreadMutexGuard<'a, R, G, U>, Self> {
        if let Some(data) = f(&mut s) {
            let data = data.into();
            let mu = s.mu;
            std::mem::forget(s);
            Ok(MappedThreadMutexGuard {
                mu,
                data,
                marker: PhantomData,
            })
        } else {
            Err(s)
        }
    }
}
impl<'a, R: RawMutex, G: GetThreadId, T: ?Sized> Deref for MappedThreadMutexGuard<'a, R, G, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { self.data.as_ref() }
    }
}
impl<'a, R: RawMutex, G: GetThreadId, T: ?Sized> DerefMut for MappedThreadMutexGuard<'a, R, G, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { self.data.as_mut() }
    }
}
impl<'a, R: RawMutex, G: GetThreadId, T: ?Sized> Drop for MappedThreadMutexGuard<'a, R, G, T> {
    fn drop(&mut self) {
        unsafe { self.mu.unlock() }
    }
}
impl<'a, R: RawMutex, G: GetThreadId, T: ?Sized + fmt::Display> fmt::Display
    for MappedThreadMutexGuard<'a, R, G, T>
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(&**self, f)
    }
}
impl<'a, R: RawMutex, G: GetThreadId, T: ?Sized + fmt::Debug> fmt::Debug
    for MappedThreadMutexGuard<'a, R, G, T>
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}
