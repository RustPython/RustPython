//! A module implementing an io type backed by the C runtime's file descriptors, i.e. what's
//! returned from libc::open, even on windows.

use std::{cmp, ffi, fmt, io};

#[cfg(not(windows))]
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};
#[cfg(windows)]
use std::os::windows::io::BorrowedHandle;

mod c {
    pub(super) use libc::*;

    #[cfg(windows)]
    pub(super) use libc::commit as fsync;
    #[cfg(windows)]
    unsafe extern "C" {
        #[link_name = "_chsize_s"]
        pub(super) fn ftruncate(fd: i32, len: i64) -> i32;
    }
}

// this is basically what CPython has for Py_off_t; windows uses long long
// for offsets, other platforms just use off_t
#[cfg(not(windows))]
pub type Offset = c::off_t;
#[cfg(windows)]
pub type Offset = c::c_longlong;

#[cfg(not(windows))]
pub type Raw = RawFd;
#[cfg(windows)]
pub type Raw = i32;

#[inline]
fn cvt<I: num_traits::PrimInt>(ret: I) -> io::Result<I> {
    if ret < I::zero() {
        Err(crate::os::last_os_error())
    } else {
        Ok(ret)
    }
}

fn cvt_fd(ret: Raw) -> io::Result<Owned> {
    cvt(ret).map(|fd| unsafe { Owned::from_raw(fd) })
}

const MAX_RW: usize = if cfg!(any(windows, target_vendor = "apple")) {
    i32::MAX as usize
} else {
    isize::MAX as usize
};

#[cfg(not(windows))]
type OwnedInner = OwnedFd;
#[cfg(not(windows))]
type BorrowedInner<'fd> = BorrowedFd<'fd>;

#[cfg(windows)]
mod win {
    use super::*;
    use std::marker::PhantomData;
    use std::mem::ManuallyDrop;

    #[repr(transparent)]
    pub(super) struct OwnedInner(i32);

    impl OwnedInner {
        #[inline]
        pub unsafe fn from_raw_fd(fd: Raw) -> Self {
            Self(fd)
        }
        #[inline]
        pub fn as_raw_fd(&self) -> Raw {
            self.0
        }
        #[inline]
        pub fn into_raw_fd(self) -> Raw {
            let me = ManuallyDrop::new(self);
            me.0
        }
    }

    impl Drop for OwnedInner {
        #[inline]
        fn drop(&mut self) {
            let _ = _close(self.0);
        }
    }

    #[derive(Copy, Clone)]
    #[repr(transparent)]
    pub(super) struct BorrowedInner<'fd> {
        fd: Raw,
        _marker: PhantomData<&'fd Owned>,
    }

    impl BorrowedInner<'_> {
        #[inline]
        pub const unsafe fn borrow_raw(fd: Raw) -> Self {
            Self {
                fd,
                _marker: PhantomData,
            }
        }
        #[inline]
        pub fn as_raw_fd(&self) -> Raw {
            self.fd
        }
    }
}

#[cfg(windows)]
use self::win::{BorrowedInner, OwnedInner};

#[repr(transparent)]
pub struct Owned {
    inner: OwnedInner,
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
    inner: BorrowedInner<'fd>,
}

impl<'fd> PartialEq for Borrowed<'fd> {
    fn eq(&self, other: &Self) -> bool {
        self.as_raw() == other.as_raw()
    }
}
impl<'fd> Eq for Borrowed<'fd> {}

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
    /// `fd` must be a valid file descriptor.
    #[inline]
    pub unsafe fn from_raw(fd: Raw) -> Self {
        let inner = unsafe { OwnedInner::from_raw_fd(fd) };
        Self { inner }
    }

    /// Create a `crt_fd::Owned` from a raw file descriptor.
    ///
    /// Returns an error if `fd` is -1.
    ///
    /// # Safety
    ///
    /// `fd` must be a valid file descriptor.
    #[inline]
    pub unsafe fn try_from_raw(fd: Raw) -> io::Result<Self> {
        if fd == -1 {
            Err(ebadf())
        } else {
            Ok(unsafe { Self::from_raw(fd) })
        }
    }

    #[inline]
    pub fn borrow(&self) -> Borrowed<'_> {
        unsafe { Borrowed::borrow_raw(self.as_raw()) }
    }

    #[inline]
    pub fn as_raw(&self) -> Raw {
        self.inner.as_raw_fd()
    }

    #[inline]
    pub fn into_raw(self) -> Raw {
        self.inner.into_raw_fd()
    }

    pub fn leak<'fd>(self) -> Borrowed<'fd> {
        unsafe { Borrowed::borrow_raw(self.into_raw()) }
    }
}

#[cfg(unix)]
impl From<Owned> for OwnedFd {
    fn from(fd: Owned) -> Self {
        fd.inner
    }
}

#[cfg(unix)]
impl From<OwnedFd> for Owned {
    fn from(fd: OwnedFd) -> Self {
        Self { inner: fd }
    }
}

#[cfg(unix)]
impl AsFd for Owned {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.inner.as_fd()
    }
}

#[cfg(unix)]
impl AsRawFd for Owned {
    fn as_raw_fd(&self) -> RawFd {
        self.as_raw()
    }
}

#[cfg(unix)]
impl FromRawFd for Owned {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        unsafe { Self::from_raw(fd) }
    }
}

#[cfg(unix)]
impl IntoRawFd for Owned {
    fn into_raw_fd(self) -> RawFd {
        self.into_raw()
    }
}

impl<'fd> Borrowed<'fd> {
    /// Create a `crt_fd::Borrowed` from a raw file descriptor.
    ///
    /// # Safety
    ///
    /// `fd` must be a valid file descriptor.
    #[inline]
    pub const unsafe fn borrow_raw(fd: Raw) -> Self {
        let inner = unsafe { BorrowedInner::borrow_raw(fd) };
        Self { inner }
    }

    /// Create a `crt_fd::Borrowed` from a raw file descriptor.
    ///
    /// Returns an error if `fd` is -1.
    ///
    /// # Safety
    ///
    /// `fd` must be a valid file descriptor.
    #[inline]
    pub unsafe fn try_borrow_raw(fd: Raw) -> io::Result<Self> {
        if fd == -1 {
            Err(ebadf())
        } else {
            Ok(unsafe { Self::borrow_raw(fd) })
        }
    }

    #[inline]
    pub fn as_raw(self) -> Raw {
        self.inner.as_raw_fd()
    }
}

#[cfg(unix)]
impl<'fd> From<Borrowed<'fd>> for BorrowedFd<'fd> {
    fn from(fd: Borrowed<'fd>) -> Self {
        fd.inner
    }
}

#[cfg(unix)]
impl<'fd> From<BorrowedFd<'fd>> for Borrowed<'fd> {
    fn from(fd: BorrowedFd<'fd>) -> Self {
        Self { inner: fd }
    }
}

#[cfg(unix)]
impl AsFd for Borrowed<'_> {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.inner.as_fd()
    }
}

#[cfg(unix)]
impl AsRawFd for Borrowed<'_> {
    fn as_raw_fd(&self) -> RawFd {
        self.as_raw()
    }
}

#[inline]
fn ebadf() -> io::Error {
    io::Error::from_raw_os_error(c::EBADF)
}

pub fn open(path: &ffi::CStr, flags: i32, mode: i32) -> io::Result<Owned> {
    cvt_fd(unsafe { c::open(path.as_ptr(), flags, mode) })
}

#[cfg(windows)]
pub fn wopen(path: &widestring::WideCStr, flags: i32, mode: i32) -> io::Result<Owned> {
    cvt_fd(unsafe { suppress_iph!(c::wopen(path.as_ptr(), flags, mode)) })
}

#[cfg(all(any(unix, target_os = "wasi"), not(target_os = "redox")))]
pub fn openat(dir: Borrowed<'_>, path: &ffi::CStr, flags: i32, mode: i32) -> io::Result<Owned> {
    cvt_fd(unsafe { c::openat(dir.as_raw(), path.as_ptr(), flags, mode) })
}

pub fn fsync(fd: Borrowed<'_>) -> io::Result<()> {
    cvt(unsafe { suppress_iph!(c::fsync(fd.as_raw())) })?;
    Ok(())
}

fn _close(fd: Raw) -> io::Result<()> {
    cvt(unsafe { suppress_iph!(c::close(fd)) })?;
    Ok(())
}

pub fn close(fd: Owned) -> io::Result<()> {
    _close(fd.into_raw())
}

pub fn ftruncate(fd: Borrowed<'_>, len: Offset) -> io::Result<()> {
    cvt(unsafe { suppress_iph!(c::ftruncate(fd.as_raw(), len)) })?;
    Ok(())
}

#[cfg(windows)]
pub fn as_handle(fd: Borrowed<'_>) -> io::Result<BorrowedHandle<'_>> {
    use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
    unsafe extern "C" {
        fn _get_osfhandle(fd: Borrowed<'_>) -> c::intptr_t;
    }
    let handle = unsafe { suppress_iph!(_get_osfhandle(fd)) };
    if handle as HANDLE == INVALID_HANDLE_VALUE {
        Err(crate::os::last_os_error())
    } else {
        Ok(unsafe { BorrowedHandle::borrow_raw(handle as _) })
    }
}

fn _write(fd: Raw, buf: &[u8]) -> io::Result<usize> {
    let count = cmp::min(buf.len(), MAX_RW);
    let n = cvt(unsafe { suppress_iph!(c::write(fd, buf.as_ptr() as _, count as _)) })?;
    Ok(n as usize)
}

fn _read(fd: Raw, buf: &mut [u8]) -> io::Result<usize> {
    let count = cmp::min(buf.len(), MAX_RW);
    let n = cvt(unsafe { suppress_iph!(libc::read(fd, buf.as_mut_ptr() as _, count as _)) })?;
    Ok(n as usize)
}

pub fn write(fd: Borrowed<'_>, buf: &[u8]) -> io::Result<usize> {
    _write(fd.as_raw(), buf)
}

pub fn read(fd: Borrowed<'_>, buf: &mut [u8]) -> io::Result<usize> {
    _read(fd.as_raw(), buf)
}

macro_rules! impl_rw {
    ($t:ty) => {
        impl io::Write for $t {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                _write(self.as_raw(), buf)
            }

            #[inline]
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        impl io::Read for $t {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                _read(self.as_raw(), buf)
            }
        }
    };
}

impl_rw!(Owned);
impl_rw!(Borrowed<'_>);
