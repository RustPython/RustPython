use parking_lot::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::ops::{Deref, DerefMut};

#[derive(Debug, Default)]
#[repr(transparent)]
pub struct PyMutex<T: ?Sized>(Mutex<T>);

impl<T> PyMutex<T> {
    pub const fn new(value: T) -> Self {
        Self(parking_lot::const_mutex(value))
    }
}

impl<T: ?Sized> PyMutex<T> {
    pub fn lock(&self) -> PyMutexGuard<T> {
        PyMutexGuard(self.0.lock())
    }
}

#[derive(Debug)]
#[repr(transparent)]
pub struct PyMutexGuard<'a, T: ?Sized>(MutexGuard<'a, T>);
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

#[derive(Debug, Default)]
#[repr(transparent)]
pub struct PyRwLock<T: ?Sized>(RwLock<T>);

impl<T> PyRwLock<T> {
    pub const fn new(value: T) -> Self {
        Self(parking_lot::const_rwlock(value))
    }
}

impl<T: ?Sized> PyRwLock<T> {
    pub fn read(&self) -> PyRwLockReadGuard<T> {
        PyRwLockReadGuard(self.0.read())
    }
    pub fn write(&self) -> PyRwLockWriteGuard<T> {
        PyRwLockWriteGuard(self.0.write())
    }
}

#[derive(Debug)]
#[repr(transparent)]
pub struct PyRwLockReadGuard<'a, T: ?Sized>(RwLockReadGuard<'a, T>);
impl<T: ?Sized> Deref for PyRwLockReadGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.0.deref()
    }
}

#[derive(Debug)]
#[repr(transparent)]
pub struct PyRwLockWriteGuard<'a, T: ?Sized>(RwLockWriteGuard<'a, T>);
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
