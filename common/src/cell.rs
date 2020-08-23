#[cfg(feature = "threading")]
pub use once_cell::sync::{Lazy, OnceCell};
#[cfg(not(feature = "threading"))]
pub use once_cell::unsync::{Lazy, OnceCell};
#[cfg(feature = "threading")]
use parking_lot::{
    MappedRwLockReadGuard, MappedRwLockWriteGuard, Mutex, MutexGuard, RwLock, RwLockReadGuard,
    RwLockWriteGuard,
};
#[cfg(not(feature = "threading"))]
use std::cell::{Ref, RefCell, RefMut};
use std::ops::{Deref, DerefMut};

cfg_if::cfg_if! {
    if #[cfg(feature = "threading")] {
        type MutexInner<T> = Mutex<T>;
        type MutexGuardInner<'a, T> = MutexGuard<'a, T>;
        const fn new_mutex<T>(value: T) -> MutexInner<T> {
            parking_lot::const_mutex(value)
        }
        fn lock_mutex<T: ?Sized>(m: &MutexInner<T>) -> MutexGuardInner<T> {
            m.lock()
        }
    } else {
        type MutexInner<T> = RefCell<T>;
        type MutexGuardInner<'a, T> = RefMut<'a, T>;
        const fn new_mutex<T>(value: T) -> MutexInner<T> {
            RefCell::new(value)
        }
        fn lock_mutex<T: ?Sized>(m: &MutexInner<T>) -> MutexGuardInner<T> {
            m.borrow_mut()
        }
    }
}

#[derive(Debug, Default)]
#[repr(transparent)]
pub struct PyMutex<T: ?Sized>(MutexInner<T>);

impl<T> PyMutex<T> {
    pub const fn new(value: T) -> Self {
        Self(new_mutex(value))
    }
}

impl<T: ?Sized> PyMutex<T> {
    pub fn lock(&self) -> PyMutexGuard<T> {
        PyMutexGuard(lock_mutex(&self.0))
    }
}

#[derive(Debug)]
#[repr(transparent)]
pub struct PyMutexGuard<'a, T: ?Sized>(MutexGuardInner<'a, T>);
impl<T: ?Sized> Deref for PyMutexGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.0.deref()
    }
}
impl<T: ?Sized> DerefMut for PyMutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.0.deref_mut()
    }
}

cfg_if::cfg_if! {
    if #[cfg(feature = "threading")] {
        type RwLockInner<T> = RwLock<T>;
        type RwLockReadInner<'a, T> = RwLockReadGuard<'a, T>;
        type MappedRwLockReadInner<'a, T> = MappedRwLockReadGuard<'a, T>;
        type RwLockWriteInner<'a, T> = RwLockWriteGuard<'a, T>;
        type MappedRwLockWriteInner<'a, T> = MappedRwLockWriteGuard<'a, T>;
        const fn new_rwlock<T>(value: T) -> RwLockInner<T> {
            parking_lot::const_rwlock(value)
        }
        fn read_rwlock<T: ?Sized>(m: &RwLockInner<T>) -> RwLockReadInner<T> {
            m.read()
        }
        fn write_rwlock<T: ?Sized>(m: &RwLockInner<T>) -> RwLockWriteInner<T> {
            m.write()
        }
    } else {
        type RwLockInner<T> = RefCell<T>;
        type RwLockReadInner<'a, T> = Ref<'a, T>;
        type MappedRwLockReadInner<'a, T> = Ref<'a, T>;
        type RwLockWriteInner<'a, T> = RefMut<'a, T>;
        type MappedRwLockWriteInner<'a, T> = RefMut<'a, T>;
        const fn new_rwlock<T>(value: T) -> RwLockInner<T> {
            RefCell::new(value)
        }
        fn read_rwlock<T: ?Sized>(m: &RwLockInner<T>) -> RwLockReadInner<T> {
            m.borrow()
        }
        fn write_rwlock<T: ?Sized>(m: &RwLockInner<T>) -> RwLockWriteInner<T> {
            m.borrow_mut()
        }
    }
}

#[derive(Debug, Default)]
#[repr(transparent)]
pub struct PyRwLock<T: ?Sized>(RwLockInner<T>);

impl<T> PyRwLock<T> {
    pub const fn new(value: T) -> Self {
        Self(new_rwlock(value))
    }
}

impl<T: ?Sized> PyRwLock<T> {
    pub fn read(&self) -> PyRwLockReadGuard<T> {
        PyRwLockReadGuard(read_rwlock(&self.0))
    }
    pub fn write(&self) -> PyRwLockWriteGuard<T> {
        PyRwLockWriteGuard(write_rwlock(&self.0))
    }
}

#[derive(Debug)]
#[repr(transparent)]
pub struct PyRwLockReadGuard<'a, T: ?Sized>(RwLockReadInner<'a, T>);
impl<T: ?Sized> Deref for PyRwLockReadGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.0.deref()
    }
}

#[repr(transparent)]
pub struct PyMappedRwLockReadGuard<'a, T: ?Sized>(MappedRwLockReadInner<'a, T>);
impl<T: ?Sized> Deref for PyMappedRwLockReadGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.0.deref()
    }
}

impl<'a, T: ?Sized> PyRwLockReadGuard<'a, T> {
    #[inline]
    pub fn map<U: ?Sized, F>(s: Self, f: F) -> PyMappedRwLockReadGuard<'a, U>
    where
        F: FnOnce(&T) -> &U,
    {
        PyMappedRwLockReadGuard(RwLockReadInner::map(s.0, f))
    }
}

#[derive(Debug)]
#[repr(transparent)]
pub struct PyRwLockWriteGuard<'a, T: ?Sized>(RwLockWriteInner<'a, T>);
impl<T: ?Sized> Deref for PyRwLockWriteGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.0.deref()
    }
}
impl<T: ?Sized> DerefMut for PyRwLockWriteGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.0.deref_mut()
    }
}

#[repr(transparent)]
pub struct PyMappedRwLockWriteGuard<'a, T: ?Sized>(MappedRwLockWriteInner<'a, T>);
impl<T: ?Sized> Deref for PyMappedRwLockWriteGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.0.deref()
    }
}
impl<T: ?Sized> DerefMut for PyMappedRwLockWriteGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.0.deref_mut()
    }
}

impl<'a, T: ?Sized> PyRwLockWriteGuard<'a, T> {
    #[inline]
    pub fn map<U: ?Sized, F>(s: Self, f: F) -> PyMappedRwLockWriteGuard<'a, U>
    where
        F: FnOnce(&mut T) -> &mut U,
    {
        PyMappedRwLockWriteGuard(RwLockWriteInner::map(s.0, f))
    }
}
