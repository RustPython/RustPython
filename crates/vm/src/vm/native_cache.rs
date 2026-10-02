//! Native attribute factories cache Python values in their owning interpreter.

use crate::common::{lock::PyMutex, rc::PyRc};
use core::any::{Any, TypeId};
use std::collections::HashMap;

#[cfg(feature = "threading")]
type Value = dyn Any + Send + Sync;
#[cfg(not(feature = "threading"))]
type Value = dyn Any;

#[derive(Default)]
pub(super) struct NativeCache {
    state: PyMutex<CacheState>,
}

#[derive(Default)]
struct CacheState {
    entries: HashMap<TypeId, PyRc<Value>>,
    closed: bool,
}

impl NativeCache {
    fn get<K: 'static, T>(&self) -> Option<T>
    where
        T: Any + Clone + crate::object::PyThreadingConstraint,
    {
        let cached = self.state.lock().entries.get(&TypeId::of::<K>()).cloned();
        cached.map(|value| {
            value
                .downcast_ref::<T>()
                .expect("native cache key reused with a different value type")
                .clone()
        })
    }

    fn replace<K: 'static, T>(&self, value: T)
    where
        T: Any + crate::object::PyThreadingConstraint,
    {
        let value: PyRc<Value> = PyRc::new(value);
        let previous = {
            let mut state = self.state.lock();
            if state.closed {
                None
            } else {
                state.entries.insert(TypeId::of::<K>(), value.clone())
            }
        };
        drop(previous);
    }

    pub(super) fn get_or_init<K: 'static, T>(&self, init: impl FnOnce() -> T) -> T
    where
        T: Any + Clone + crate::object::PyThreadingConstraint,
    {
        self.get_or_try_init::<K, T, core::convert::Infallible>(|| Ok(init()))
            .unwrap_or_else(|never| match never {})
    }

    fn get_or_try_init<K: 'static, T, E>(&self, init: impl FnOnce() -> Result<T, E>) -> Result<T, E>
    where
        T: Any + Clone + crate::object::PyThreadingConstraint,
    {
        let key = TypeId::of::<K>();
        let cached = self.state.lock().entries.get(&key).cloned();
        let value = if let Some(value) = cached {
            value
        } else {
            // A factory may allocate, run GC, and reenter this same key from a
            // finalizer. Initialize without locks and publish a single winner.
            let candidate: PyRc<Value> = PyRc::new(init()?);
            let winner = {
                let mut state = self.state.lock();
                if state.closed {
                    candidate.clone()
                } else {
                    state
                        .entries
                        .entry(key)
                        .or_insert_with(|| candidate.clone())
                        .clone()
                }
            };
            // Neither losing Python values nor arbitrary T::clone code may run
            // while the directory is locked.
            drop(candidate);
            winner
        };
        Ok(value
            .downcast_ref::<T>()
            .expect("native cache key reused with a different value type")
            .clone())
    }

    /// The owner must be entered. Finalizers may still call native factories,
    /// but cannot reinstall roots into a cache whose interpreter is retiring.
    pub(super) fn close(&self) {
        let roots = {
            let mut state = self.state.lock();
            state.closed = true;
            core::mem::take(&mut state.entries)
        };
        drop(roots);
    }

    #[cfg(all(unix, feature = "threading"))]
    pub(super) unsafe fn reinit_after_fork(&self) {
        unsafe { crate::common::lock::reinit_mutex_after_fork(&self.state) };
    }
}

impl super::VirtualMachine {
    /// Read a replaceable native cache entry without running clone under its lock.
    #[doc(hidden)]
    pub fn __get_native<K: 'static, T>(&self) -> Option<T>
    where
        T: Any + Clone + crate::object::PyThreadingConstraint,
    {
        self.state.native_cache.get::<K, T>()
    }

    /// Publish an immutable snapshot, releasing the previous value outside locks.
    #[doc(hidden)]
    pub fn __replace_native<K: 'static, T>(&self, value: T)
    where
        T: Any + crate::object::PyThreadingConstraint,
    {
        self.state.native_cache.replace::<K, T>(value);
    }

    /// Used by `#[pyattr(once)]` to preserve identity within one interpreter.
    /// Each factory must use its own key type and be idempotent: competing or
    /// reentrant calls may construct multiple candidates before one is cached.
    #[doc(hidden)]
    pub fn __cached_native<K: 'static, T>(&self, init: impl FnOnce() -> T) -> T
    where
        T: Any + Clone + crate::object::PyThreadingConstraint,
    {
        self.state.native_cache.get_or_init::<K, T>(init)
    }

    /// Fallible native factories do not cache an unsuccessful initialization.
    #[doc(hidden)]
    pub fn __try_cached_native<K: 'static, T, E>(
        &self,
        init: impl FnOnce() -> Result<T, E>,
    ) -> Result<T, E>
    where
        T: Any + Clone + crate::object::PyThreadingConstraint,
    {
        self.state.native_cache.get_or_try_init::<K, T, E>(init)
    }
}

#[cfg(all(unix, feature = "threading"))]
impl super::PyGlobalState {
    pub(crate) unsafe fn reinit_native_cache_after_fork(&self) {
        unsafe {
            self.native_cache.reinit_after_fork();
            self.native_types.reinit_after_fork();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reentry_publishes_one_identity_and_errors_are_retryable() {
        struct Key;
        let cache = NativeCache::default();
        assert_eq!(
            cache.get_or_try_init::<Key, String, _>(|| Err("retry")),
            Err("retry")
        );
        let value = cache.get_or_init::<Key, _>(|| {
            assert_eq!(cache.get_or_init::<Key, _>(|| "inner".to_owned()), "inner");
            "outer".to_owned()
        });
        assert_eq!(value, "inner");
        cache.close();
        assert_eq!(
            cache.get_or_init::<Key, _>(|| "transient".to_owned()),
            "transient"
        );
        assert!(cache.state.lock().entries.is_empty());
    }

    #[test]
    fn replacement_and_close_release_snapshots() {
        struct Key;
        let cache = NativeCache::default();
        let first = PyRc::new(());
        let weak = PyRc::downgrade(&first);
        cache.replace::<Key, _>(first);
        let second = PyRc::new(());
        cache.replace::<Key, _>(second.clone());
        assert!(weak.upgrade().is_none());
        assert!(PyRc::ptr_eq(
            &cache.get::<Key, PyRc<()>>().unwrap(),
            &second
        ));
        cache.close();
        cache.replace::<Key, _>(second);
        assert!(cache.get::<Key, PyRc<()>>().is_none());
    }

    #[cfg(feature = "threading")]
    #[test]
    fn concurrent_factories_keep_the_same_winner() {
        struct Key;
        let cache = NativeCache::default();
        let barrier = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            let make = || {
                cache.get_or_init::<Key, _>(|| {
                    barrier.wait();
                    PyRc::new(())
                })
            };
            let first = scope.spawn(make);
            let second = scope.spawn(make);
            assert!(PyRc::ptr_eq(
                &first.join().unwrap(),
                &second.join().unwrap()
            ));
        });
    }
}
