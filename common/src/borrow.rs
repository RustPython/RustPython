use crate::lock::{
    MapImmutable, PyImmutableMappedMutexGuard, PyMappedMutexGuard, PyMappedRwLockReadGuard,
    PyMappedRwLockWriteGuard, PyMutexGuard, PyRwLockReadGuard, PyRwLockWriteGuard,
};
use std::{
    fmt,
    ops::{Deref, DerefMut},
};

macro_rules! impl_from {
    ($lt:lifetime, $gen:ident, $t:ty, $($var:ident($from:ty),)*) => {
        $(
            impl<$lt, $gen: ?Sized> From<$from> for $t {
                fn from(t: $from) -> Self {
                    Self::$var(t)
                }
            }
        )*
    };
}

#[derive(Debug)]
pub enum BorrowedValue<'a, T: ?Sized> {
    Ref(&'a T),
    MuLock(PyMutexGuard<'a, T>),
    MappedMuLock(PyImmutableMappedMutexGuard<'a, T>),
    ReadLock(PyRwLockReadGuard<'a, T>),
    MappedReadLock(PyMappedRwLockReadGuard<'a, T>),
}
impl_from!('a, T, BorrowedValue<'a, T>,
    Ref(&'a T),
    MuLock(PyMutexGuard<'a, T>),
    MappedMuLock(PyImmutableMappedMutexGuard<'a, T>),
    ReadLock(PyRwLockReadGuard<'a, T>),
    MappedReadLock(PyMappedRwLockReadGuard<'a, T>),
);

impl<'a, T: ?Sized> BorrowedValue<'a, T> {
    pub fn map<U: ?Sized, F>(s: Self, f: F) -> BorrowedValue<'a, U>
    where
        F: FnOnce(&T) -> &U,
    {
        match s {
            Self::Ref(r) => BorrowedValue::Ref(f(r)),
            Self::MuLock(m) => BorrowedValue::MappedMuLock(PyMutexGuard::map_immutable(m, f)),
            Self::MappedMuLock(m) => {
                BorrowedValue::MappedMuLock(PyImmutableMappedMutexGuard::map(m, f))
            }
            Self::ReadLock(r) => BorrowedValue::MappedReadLock(PyRwLockReadGuard::map(r, f)),
            Self::MappedReadLock(m) => {
                BorrowedValue::MappedReadLock(PyMappedRwLockReadGuard::map(m, f))
            }
        }
    }
}

impl<T: ?Sized> Deref for BorrowedValue<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        match self {
            Self::Ref(r) => r,
            Self::MuLock(m) => m,
            Self::MappedMuLock(m) => m,
            Self::ReadLock(r) => r,
            Self::MappedReadLock(m) => m,
        }
    }
}

impl fmt::Display for BorrowedValue<'_, str> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(self.deref(), f)
    }
}

#[derive(Debug)]
pub enum BorrowedValueMut<'a, T: ?Sized> {
    RefMut(&'a mut T),
    MuLock(PyMutexGuard<'a, T>),
    MappedMuLock(PyMappedMutexGuard<'a, T>),
    WriteLock(PyRwLockWriteGuard<'a, T>),
    MappedWriteLock(PyMappedRwLockWriteGuard<'a, T>),
}
impl_from!('a, T, BorrowedValueMut<'a, T>,
    RefMut(&'a mut T),
    MuLock(PyMutexGuard<'a, T>),
    MappedMuLock(PyMappedMutexGuard<'a, T>),
    WriteLock(PyRwLockWriteGuard<'a, T>),
    MappedWriteLock(PyMappedRwLockWriteGuard<'a, T>),
);

impl<'a, T: ?Sized> BorrowedValueMut<'a, T> {
    pub fn map<U: ?Sized, F>(s: Self, f: F) -> BorrowedValueMut<'a, U>
    where
        F: FnOnce(&mut T) -> &mut U,
    {
        match s {
            Self::RefMut(r) => BorrowedValueMut::RefMut(f(r)),
            Self::MuLock(m) => BorrowedValueMut::MappedMuLock(PyMutexGuard::map(m, f)),
            Self::MappedMuLock(m) => BorrowedValueMut::MappedMuLock(PyMappedMutexGuard::map(m, f)),
            Self::WriteLock(r) => BorrowedValueMut::MappedWriteLock(PyRwLockWriteGuard::map(r, f)),
            Self::MappedWriteLock(m) => {
                BorrowedValueMut::MappedWriteLock(PyMappedRwLockWriteGuard::map(m, f))
            }
        }
    }
}

impl<T: ?Sized> Deref for BorrowedValueMut<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        match self {
            Self::RefMut(r) => r,
            Self::MuLock(m) => m,
            Self::MappedMuLock(m) => m,
            Self::WriteLock(w) => w,
            Self::MappedWriteLock(w) => w,
        }
    }
}

impl<T: ?Sized> DerefMut for BorrowedValueMut<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        match self {
            Self::RefMut(r) => r,
            Self::MuLock(m) => &mut *m,
            Self::MappedMuLock(m) => &mut *m,
            Self::WriteLock(w) => &mut *w,
            Self::MappedWriteLock(w) => &mut *w,
        }
    }
}
