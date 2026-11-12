#![allow(clippy::needless_lifetimes)]

use alloc::fmt;
use core::{
    cell::{OnceCell, UnsafeCell},
    marker::PhantomData,
    ops::{Deref, DerefMut},
    ptr::NonNull,
    sync::atomic::{AtomicUsize, Ordering},
};
use lock_api::{GetThreadId, GuardNoSend, RawMutex, RawMutexTimed};

// based off ReentrantMutex from lock_api

/// A mutex type that knows when it would deadlock
pub struct RawThreadMutex<R: RawMutex, G: GetThreadId> {
    owner: AtomicUsize,
    mutex: R,
    get_thread_id: G,
}

impl<R: RawMutex, G: GetThreadId> RawThreadMutex<R, G> {
    #[allow(
        clippy::declare_interior_mutable_const,
        reason = "const initializer for lock primitive contains atomics by design"
    )]
    pub const INIT: Self = Self {
        owner: AtomicUsize::new(0),
        mutex: R::INIT,
        get_thread_id: G::INIT,
    };

    #[inline]
    fn lock_internal<F: FnOnce() -> bool>(&self, try_lock: F) -> Option<bool> {
        let id = self.get_thread_id.nonzero_thread_id().get();
        if self.owner.load(Ordering::Relaxed) == id {
            return None;
        }
        if !try_lock() {
            return Some(false);
        }
        self.owner.store(id, Ordering::Relaxed);
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

    fn lock_wrapped_internal(
        &self,
        try_lock: impl Fn() -> bool,
        wrap_fn: impl FnOnce(&dyn Fn() -> bool) -> bool,
    ) -> Option<bool> {
        let id = self.get_thread_id.nonzero_thread_id().get();
        if self.owner.load(Ordering::Relaxed) == id {
            return None;
        }
        // Keep ownership across the rest of the wrapper, including unwinding.
        // Cache the result so repeated callback calls cannot acquire twice.
        let result = OnceCell::new();
        let keep_lock = wrap_fn(&|| {
            result
                .get_or_init(|| match self.lock_internal(&try_lock) {
                    Some(true) => Ok(RawThreadMutexGuard {
                        mutex: self,
                        marker: PhantomData,
                    }),
                    Some(false) => Err(TryLockThreadError::Other),
                    None => Err(TryLockThreadError::Current),
                })
                .is_ok()
        });
        match result.into_inner() {
            Some(Ok(guard)) if keep_lock => {
                core::mem::forget(guard);
                Some(true)
            }
            Some(Err(TryLockThreadError::Current)) => None,
            _ => Some(false),
        }
    }

    /// Like `lock()` but wraps the blocking wait in `wrap_fn`.
    /// The caller can use this to detach thread state while waiting.
    /// Skipping the callback returns false; repeated calls reuse its result.
    /// If the wrapper panics after acquisition, the mutex is unlocked.
    pub fn lock_wrapped<F: FnOnce(&dyn Fn())>(&self, wrap_fn: F) -> bool {
        self.lock_wrapped_internal(
            || {
                self.mutex.lock();
                true
            },
            |wait| {
                wrap_fn(&|| {
                    wait();
                });
                true
            },
        ) == Some(true)
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
        unsafe { self.mutex.unlock() };
    }
}

impl<R: RawMutex, G: GetThreadId> RawThreadMutex<R, G> {
    /// Reset this mutex to its initial (unlocked, unowned) state after `fork()`.
    ///
    /// # Safety
    ///
    /// Must only be called from the single-threaded child process immediately
    /// after `fork()`, before any other thread is created.
    #[cfg(unix)]
    pub unsafe fn reinit_after_fork(&self) {
        self.owner.store(0, Ordering::Relaxed);
        unsafe {
            let mutex_ptr = &self.mutex as *const R as *mut u8;
            core::ptr::write_bytes(mutex_ptr, 0, core::mem::size_of::<R>());
        }
    }
}

unsafe impl<R: RawMutex + Send, G: GetThreadId + Send> Send for RawThreadMutex<R, G> {}
unsafe impl<R: RawMutex + Sync, G: GetThreadId + Sync> Sync for RawThreadMutex<R, G> {}

struct RawThreadMutexGuard<'a, R: RawMutex, G: GetThreadId> {
    mutex: &'a RawThreadMutex<R, G>,
    marker: PhantomData<GuardNoSend>,
}

impl<R: RawMutex, G: GetThreadId> Drop for RawThreadMutexGuard<'_, R, G> {
    fn drop(&mut self) {
        // SAFETY: This guard is created only after acquiring the mutex.
        unsafe { self.mutex.unlock() }
    }
}

pub struct ThreadMutex<R: RawMutex, G: GetThreadId, T: ?Sized> {
    raw: RawThreadMutex<R, G>,
    data: UnsafeCell<T>,
}

impl<R: RawMutex, G: GetThreadId, T> ThreadMutex<R, G, T> {
    pub const fn new(val: T) -> Self {
        Self {
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
impl<R: RawMutex, G: GetThreadId, T> From<T> for ThreadMutex<R, G, T> {
    fn from(val: T) -> Self {
        Self::new(val)
    }
}
impl<R: RawMutex, G: GetThreadId, T: ?Sized> ThreadMutex<R, G, T> {
    /// Access the underlying raw thread mutex.
    pub fn raw(&self) -> &RawThreadMutex<R, G> {
        &self.raw
    }

    pub fn lock(&self) -> Option<ThreadMutexGuard<'_, R, G, T>> {
        if self.raw.lock() {
            Some(ThreadMutexGuard {
                mu: self,
                marker: PhantomData,
            })
        } else {
            None
        }
    }

    /// Like `lock()` but wraps the blocking wait in `wrap_fn`.
    /// The caller can use this to detach thread state while waiting.
    /// Skipping the callback returns None; repeated calls reuse its result.
    /// If the wrapper panics after acquisition, the mutex is unlocked.
    pub fn lock_wrapped<F: FnOnce(&dyn Fn())>(
        &self,
        wrap_fn: F,
    ) -> Option<ThreadMutexGuard<'_, R, G, T>> {
        if self.raw.lock_wrapped(wrap_fn) {
            Some(ThreadMutexGuard {
                mu: self,
                marker: PhantomData,
            })
        } else {
            None
        }
    }

    pub fn try_lock(&self) -> Result<ThreadMutexGuard<'_, R, G, T>, TryLockThreadError> {
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

impl<R: RawMutexTimed, G: GetThreadId, T: ?Sized> ThreadMutex<R, G, T> {
    /// Wait at most `timeout`, wrapping only the blocking acquisition.
    /// The callback attempts acquisition once; repeated calls reuse its result.
    /// Both the callback and wrapper must return true to retain the lock.
    /// Skipping the callback returns an error, and unwinding unlocks the mutex.
    pub fn try_lock_for_wrapped<F: FnOnce(&dyn Fn() -> bool) -> bool>(
        &self,
        timeout: R::Duration,
        wrap_fn: F,
    ) -> Result<ThreadMutexGuard<'_, R, G, T>, TryLockThreadError>
    where
        R::Duration: Copy,
    {
        match self
            .raw
            .lock_wrapped_internal(|| self.raw.mutex.try_lock_for(timeout), wrap_fn)
        {
            Some(true) => Ok(ThreadMutexGuard {
                mu: self,
                marker: PhantomData,
            }),
            Some(false) => Err(TryLockThreadError::Other),
            None => Err(TryLockThreadError::Current),
        }
    }
}

#[derive(Clone, Copy)]
pub enum TryLockThreadError {
    /// Acquisition failed, or the wrapper skipped or rejected it.
    Other,
    /// Failed to lock because mutex was already locked on current thread.
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
        core::mem::forget(s);
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
            core::mem::forget(s);
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
impl<R: RawMutex, G: GetThreadId, T: ?Sized> Deref for ThreadMutexGuard<'_, R, G, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.mu.data.get() }
    }
}
impl<R: RawMutex, G: GetThreadId, T: ?Sized> DerefMut for ThreadMutexGuard<'_, R, G, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.mu.data.get() }
    }
}
impl<R: RawMutex, G: GetThreadId, T: ?Sized> Drop for ThreadMutexGuard<'_, R, G, T> {
    fn drop(&mut self) {
        unsafe { self.mu.raw.unlock() }
    }
}
impl<R: RawMutex, G: GetThreadId, T: ?Sized + fmt::Display> fmt::Display
    for ThreadMutexGuard<'_, R, G, T>
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&**self, f)
    }
}
impl<R: RawMutex, G: GetThreadId, T: ?Sized + fmt::Debug> fmt::Debug
    for ThreadMutexGuard<'_, R, G, T>
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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
        core::mem::forget(s);
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
            core::mem::forget(s);
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
impl<R: RawMutex, G: GetThreadId, T: ?Sized> Deref for MappedThreadMutexGuard<'_, R, G, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { self.data.as_ref() }
    }
}
impl<R: RawMutex, G: GetThreadId, T: ?Sized> DerefMut for MappedThreadMutexGuard<'_, R, G, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { self.data.as_mut() }
    }
}
impl<R: RawMutex, G: GetThreadId, T: ?Sized> Drop for MappedThreadMutexGuard<'_, R, G, T> {
    fn drop(&mut self) {
        unsafe { self.mu.unlock() }
    }
}
impl<R: RawMutex, G: GetThreadId, T: ?Sized + fmt::Display> fmt::Display
    for MappedThreadMutexGuard<'_, R, G, T>
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&**self, f)
    }
}
impl<R: RawMutex, G: GetThreadId, T: ?Sized + fmt::Debug> fmt::Debug
    for MappedThreadMutexGuard<'_, R, G, T>
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

#[cfg(test)]
#[cfg(feature = "std")]
mod tests {
    use super::TryLockThreadError;
    use crate::lock::PyThreadMutex;
    #[cfg(feature = "threading")]
    use core::time::Duration;

    #[test]
    #[cfg(feature = "threading")]
    fn timed_lock_preserves_ownership() {
        let mutex = PyThreadMutex::new(0);
        let guard = mutex.lock().unwrap();
        assert!(matches!(
            mutex.try_lock_for_wrapped(Duration::ZERO, |_| panic!("reentrant wait")),
            Err(TryLockThreadError::Current)
        ));
        std::thread::scope(|scope| {
            scope.spawn(|| {
                assert!(matches!(
                    mutex.try_lock_for_wrapped(Duration::from_millis(1), |wait| wait()),
                    Err(TryLockThreadError::Other)
                ));
            });
        });
        drop(guard);
        let mut guard = mutex
            .try_lock_for_wrapped(Duration::ZERO, |wait| wait())
            .unwrap_or_else(|_| panic!("unlocked mutex"));
        *guard = 1;
        drop(guard);
        assert_eq!(*mutex.lock().unwrap(), 1);
    }

    #[cfg(feature = "threading")]
    fn assert_other<T>(result: Result<T, TryLockThreadError>) {
        match result {
            Err(TryLockThreadError::Other) => {}
            Err(TryLockThreadError::Current) => panic!("unexpected reentrant acquisition"),
            Ok(guard) => {
                // A regression may fabricate a guard without acquiring the raw
                // mutex. Do not unlock through that guard while reporting it.
                core::mem::forget(guard);
                panic!("unexpected successful acquisition");
            }
        }
    }

    #[test]
    fn wrapped_lock_requires_callback() {
        let mutex = PyThreadMutex::new(0);
        assert!(!mutex.raw().lock_wrapped(|_| {}));
        assert!(mutex.try_lock().is_ok());
    }

    #[test]
    fn wrapped_lock_reuses_acquisition() {
        let mutex = PyThreadMutex::new(0);
        let guard = mutex.lock_wrapped(|wait| {
            wait();
            assert!(matches!(mutex.try_lock(), Err(TryLockThreadError::Current)));
            assert!(
                mutex
                    .lock_wrapped(|_| panic!("reentrant wrapper"))
                    .is_none()
            );
            wait();
        });
        assert!(guard.is_some());
        drop(guard);
        assert!(mutex.try_lock().is_ok());
    }

    #[test]
    fn wrapped_lock_releases_on_panic() {
        use core::panic::AssertUnwindSafe;
        use std::panic::catch_unwind;

        for acquire in [false, true] {
            let mutex = PyThreadMutex::new(0);
            let result = catch_unwind(AssertUnwindSafe(|| {
                mutex.lock_wrapped(|wait| {
                    if acquire {
                        wait();
                    }
                    panic!("wrapper panic");
                })
            }));
            assert!(result.is_err());
            assert!(mutex.try_lock().is_ok());
        }
    }

    #[test]
    #[cfg(feature = "threading")]
    fn timed_wrapper_cannot_fabricate_acquisition() {
        let mutex = PyThreadMutex::new(0);
        for reported_success in [false, true] {
            assert_other(mutex.try_lock_for_wrapped(Duration::ZERO, |_| reported_success));
            assert!(mutex.try_lock().is_ok());
        }
        let guard = mutex.lock().unwrap();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                assert_other(mutex.try_lock_for_wrapped(Duration::ZERO, |wait| {
                    assert!(!wait());
                    true
                }));
                assert_other(mutex.try_lock());
            });
        });
        drop(guard);
        assert!(mutex.try_lock().is_ok());
    }

    #[test]
    #[cfg(feature = "threading")]
    fn timed_wrapper_reuses_acquisition() {
        let mutex = PyThreadMutex::new(0);
        let guard = mutex
            .try_lock_for_wrapped(Duration::ZERO, |wait| {
                assert!(wait());
                assert!(matches!(mutex.try_lock(), Err(TryLockThreadError::Current)));
                assert!(wait());
                true
            })
            .unwrap_or_else(|_| panic!("unlocked mutex"));
        drop(guard);
        assert!(mutex.try_lock().is_ok());
    }

    #[test]
    #[cfg(feature = "threading")]
    fn timed_wrapper_reuses_timeout() {
        let mutex = PyThreadMutex::new(0);
        let guard = mutex.lock().unwrap();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let (released_tx, released_rx) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            let mutex = &mutex;
            scope.spawn(move || {
                assert_other(mutex.try_lock_for_wrapped(Duration::ZERO, |wait| {
                    assert!(!wait());
                    release_tx.send(()).unwrap();
                    released_rx.recv().unwrap();
                    assert!(!wait());
                    false
                }));
                assert!(mutex.try_lock().is_ok());
            });
            release_rx.recv().unwrap();
            drop(guard);
            released_tx.send(()).unwrap();
        });
    }

    #[test]
    #[cfg(feature = "threading")]
    fn timed_wrapper_rechecks_ownership() {
        let mutex = PyThreadMutex::new(0);
        let mut guard = None;
        let result = mutex.try_lock_for_wrapped(Duration::ZERO, |wait| {
            guard = mutex.lock();
            assert!(!wait());
            true
        });
        let reentrant = matches!(result, Err(TryLockThreadError::Current));
        // Do not drop a fabricated guard on the unfixed implementation.
        core::mem::forget(result);
        assert!(reentrant);
        drop(guard);
        assert!(mutex.try_lock().is_ok());
    }

    #[test]
    #[cfg(feature = "threading")]
    fn timed_wrapper_releases_rejected_acquisition() {
        let mutex = PyThreadMutex::new(0);
        assert_other(mutex.try_lock_for_wrapped(Duration::ZERO, |wait| {
            assert!(wait());
            false
        }));
        assert!(mutex.try_lock().is_ok());
    }

    #[test]
    #[cfg(feature = "threading")]
    fn timed_wrapper_releases_on_panic() {
        use core::panic::AssertUnwindSafe;
        use std::panic::catch_unwind;

        for acquire in [false, true] {
            let mutex = PyThreadMutex::new(0);
            let result = catch_unwind(AssertUnwindSafe(|| {
                mutex.try_lock_for_wrapped(Duration::ZERO, |wait| {
                    if acquire {
                        assert!(wait());
                    }
                    panic!("wrapper panic");
                })
            }));
            assert!(result.is_err());
            assert!(mutex.try_lock().is_ok());
        }
    }
}
