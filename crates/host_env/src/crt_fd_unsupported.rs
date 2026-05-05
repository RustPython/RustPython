use alloc::fmt;
use core::marker::PhantomData;
use std::{ffi, io};

pub type Offset = i64;
pub type Raw = i32;

const EBADF: i32 = 9;

#[repr(transparent)]
pub struct Owned {
    fd: Raw,
}

impl fmt::Debug for Owned {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("crt_fd::Owned")
            .field(&self.as_raw())
            .finish()
    }
}

#[derive(Copy, Clone)]
#[repr(transparent)]
pub struct Borrowed<'fd> {
    fd: Raw,
    _marker: PhantomData<&'fd Owned>,
}

impl PartialEq for Borrowed<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.as_raw() == other.as_raw()
    }
}

impl Eq for Borrowed<'_> {}

impl fmt::Debug for Borrowed<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("crt_fd::Borrowed")
            .field(&self.as_raw())
            .finish()
    }
}

impl Owned {
    /// Create a `crt_fd::Owned` from a raw file descriptor.
    ///
    /// # Safety
    ///
    /// `fd` must be a valid file descriptor for the embedding host.
    #[inline]
    pub const unsafe fn from_raw(fd: Raw) -> Self {
        Self { fd }
    }

    /// Create a `crt_fd::Owned` from a raw file descriptor.
    ///
    /// Returns an error if `fd` is negative.
    ///
    /// # Safety
    ///
    /// `fd` must be a valid file descriptor for the embedding host.
    #[inline]
    pub unsafe fn try_from_raw(fd: Raw) -> io::Result<Self> {
        if fd < 0 {
            Err(ebadf())
        } else {
            Ok(unsafe { Self::from_raw(fd) })
        }
    }

    #[inline]
    pub const fn borrow(&self) -> Borrowed<'_> {
        unsafe { Borrowed::borrow_raw(self.as_raw()) }
    }

    #[inline]
    pub const fn as_raw(&self) -> Raw {
        self.fd
    }

    #[inline]
    pub fn into_raw(self) -> Raw {
        let fd = self.fd;
        core::mem::forget(self);
        fd
    }

    pub fn leak<'fd>(self) -> Borrowed<'fd> {
        unsafe { Borrowed::borrow_raw(self.into_raw()) }
    }
}

impl Drop for Owned {
    fn drop(&mut self) {}
}

impl<'fd> Borrowed<'fd> {
    /// Create a `crt_fd::Borrowed` from a raw file descriptor.
    ///
    /// # Safety
    ///
    /// `fd` must be a valid file descriptor for the embedding host.
    #[inline]
    pub const unsafe fn borrow_raw(fd: Raw) -> Self {
        Self {
            fd,
            _marker: PhantomData,
        }
    }

    /// Create a `crt_fd::Borrowed` from a raw file descriptor.
    ///
    /// Returns an error if `fd` is negative.
    ///
    /// # Safety
    ///
    /// `fd` must be a valid file descriptor for the embedding host.
    #[inline]
    pub unsafe fn try_borrow_raw(fd: Raw) -> io::Result<Self> {
        if fd < 0 {
            Err(ebadf())
        } else {
            Ok(unsafe { Self::borrow_raw(fd) })
        }
    }

    #[inline]
    pub const fn as_raw(self) -> Raw {
        self.fd
    }
}

#[inline]
fn ebadf() -> io::Error {
    io::Error::from_raw_os_error(EBADF)
}

pub fn open(_path: &ffi::CStr, _flags: i32, _mode: i32) -> io::Result<Owned> {
    Err(unsupported())
}

pub fn openat(_dir: Borrowed<'_>, _path: &ffi::CStr, _flags: i32, _mode: i32) -> io::Result<Owned> {
    Err(unsupported())
}

pub fn fsync(_fd: Borrowed<'_>) -> io::Result<()> {
    Err(ebadf())
}

pub fn close(_fd: Owned) -> io::Result<()> {
    Err(ebadf())
}

pub fn ftruncate(_fd: Borrowed<'_>, _len: Offset) -> io::Result<()> {
    Err(ebadf())
}

pub fn write(_fd: Borrowed<'_>, _buf: &[u8]) -> io::Result<usize> {
    Err(ebadf())
}

pub fn read(_fd: Borrowed<'_>, _buf: &mut [u8]) -> io::Result<usize> {
    Err(ebadf())
}

fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "host file descriptors are unsupported on this platform",
    )
}
