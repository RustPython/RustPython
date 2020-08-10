use crate::cell::{
    PyMappedMutexGuard, PyMappedRwLockReadGuard, PyMappedRwLockWriteGuard, PyMutexGuard,
    PyRwLockReadGuard, PyRwLockWriteGuard,
};
use std::ops::{Deref, DerefMut};

pub trait BorrowValue<'a> {
    type Borrowed: 'a + Deref;
    fn borrow_value(&'a self) -> Self::Borrowed;
}

#[derive(Debug, derive_more::From)]
pub enum BorrowedValue<'a, T: ?Sized> {
    Ref(&'a T),
    MuLock(PyMutexGuard<'a, T>),
    MappedMuLock(PyMappedMutexGuard<'a, T>),
    ReadLock(PyRwLockReadGuard<'a, T>),
    MappedReadLock(PyMappedRwLockReadGuard<'a, T>),
}

impl<T: ?Sized> Deref for BorrowedValue<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        match self {
            Self::Ref(r) => r,
            Self::MuLock(m) => &m,
            Self::MappedMuLock(m) => &m,
            Self::ReadLock(r) => &r,
            Self::MappedReadLock(m) => &m,
        }
    }
}

#[derive(Debug, derive_more::From)]
pub enum BorrowedValueMut<'a, T: ?Sized> {
    RefMut(&'a mut T),
    MuLock(PyMutexGuard<'a, T>),
    MappedMuLock(PyMappedMutexGuard<'a, T>),
    WriteLock(PyRwLockWriteGuard<'a, T>),
    MappedWriteLock(PyMappedRwLockWriteGuard<'a, T>),
}

impl<T: ?Sized> Deref for BorrowedValueMut<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        match self {
            Self::RefMut(r) => r,
            Self::MuLock(m) => &m,
            Self::MappedMuLock(m) => &m,
            Self::WriteLock(w) => &w,
            Self::MappedWriteLock(w) => &w,
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
