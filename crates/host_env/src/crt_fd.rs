//! A module implementing an io type backed by the C runtime's file descriptors, i.e. what's
//! returned from libc::open, even on windows.

use alloc::fmt;
use core::cmp;
use std::{ffi, io};

#[cfg(any(unix, target_os = "wasi"))]
use std::os::fd::AsFd;
#[cfg(not(windows))]
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};
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
pub type Offset = cfg_select! {
    windows => c::c_longlong,
    _ => c::off_t,
};

pub type Raw = cfg_select! {
    windows => i32,
    _ => RawFd,
};

#[inline]
fn cvt<I: num_traits::PrimInt>(ret: I) -> io::Result<I> {
    if ret < I::zero() {
        // CRT functions set errno, not GetLastError(), so use errno_io_error
        Err(crate::os::errno_io_error())
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
    use core::marker::PhantomData;
    use core::mem::ManuallyDrop;

    #[repr(transparent)]
    pub(super) struct OwnedInner(i32);

    impl OwnedInner {
        #[inline]
        pub(super) unsafe fn from_raw_fd(fd: Raw) -> Self {
            Self(fd)
        }

        #[inline]
        pub(super) fn as_raw_fd(&self) -> Raw {
            self.0
        }

        #[inline]
        pub(super) fn into_raw_fd(self) -> Raw {
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
        pub(super) const unsafe fn borrow_raw(fd: Raw) -> Self {
            Self {
                fd,
                _marker: PhantomData,
            }
        }

        #[inline]
        pub(super) fn as_raw_fd(self) -> Raw {
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
    /// `fd` must be a valid file descriptor.
    #[inline]
    #[must_use]
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
    #[must_use]
    pub fn borrow(&self) -> Borrowed<'_> {
        unsafe { Borrowed::borrow_raw(self.as_raw()) }
    }

    #[inline]
    #[must_use]
    pub fn as_raw(&self) -> Raw {
        self.inner.as_raw_fd()
    }

    #[inline]
    #[must_use]
    pub fn into_raw(self) -> Raw {
        self.inner.into_raw_fd()
    }

    #[must_use]
    pub fn leak<'fd>(self) -> Borrowed<'fd> {
        unsafe { Borrowed::borrow_raw(self.into_raw()) }
    }
}

#[cfg(any(unix, target_os = "wasi"))]
impl From<Owned> for OwnedFd {
    fn from(fd: Owned) -> Self {
        fd.inner
    }
}

#[cfg(any(unix, target_os = "wasi"))]
impl From<OwnedFd> for Owned {
    fn from(fd: OwnedFd) -> Self {
        Self { inner: fd }
    }
}

#[cfg(any(unix, target_os = "wasi"))]
impl AsFd for Owned {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.inner.as_fd()
    }
}

#[cfg(any(unix, target_os = "wasi"))]
impl AsRawFd for Owned {
    fn as_raw_fd(&self) -> RawFd {
        self.as_raw()
    }
}

#[cfg(any(unix, target_os = "wasi"))]
impl FromRawFd for Owned {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        unsafe { Self::from_raw(fd) }
    }
}

#[cfg(any(unix, target_os = "wasi"))]
impl IntoRawFd for Owned {
    fn into_raw_fd(self) -> RawFd {
        self.into_raw()
    }
}

impl Borrowed<'_> {
    /// Create a `crt_fd::Borrowed` from a raw file descriptor.
    ///
    /// # Safety
    ///
    /// `fd` must be a valid file descriptor.
    #[inline]
    #[must_use]
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
    #[must_use]
    pub fn as_raw(self) -> Raw {
        self.inner.as_raw_fd()
    }
}

#[cfg(any(unix, target_os = "wasi"))]
impl<'fd> From<Borrowed<'fd>> for BorrowedFd<'fd> {
    fn from(fd: Borrowed<'fd>) -> Self {
        fd.inner
    }
}

#[cfg(any(unix, target_os = "wasi"))]
impl<'fd> From<BorrowedFd<'fd>> for Borrowed<'fd> {
    fn from(fd: BorrowedFd<'fd>) -> Self {
        Self { inner: fd }
    }
}

#[cfg(any(unix, target_os = "wasi"))]
impl AsFd for Borrowed<'_> {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.inner.as_fd()
    }
}

#[cfg(any(unix, target_os = "wasi"))]
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

#[cfg(not(any(target_os = "freebsd", target_os = "linux")))]
pub fn closerange(fd_low: Raw, fd_high: Raw) {
    close_range_slow(fd_low, fd_high);
}

// _Py_closerange
#[cfg(any(target_os = "freebsd", target_os = "linux"))]
pub fn closerange(fd_low: Raw, fd_high: Raw) {
    // CPython clamps low to 0.
    let fd_low = fd_low.max(0);
    // close_range(2) is [low, high] whereas CPython's is [low, high).
    if fd_high < fd_low {
        return;
    }
    let high = fd_high - 1;

    if close_range(fd_low as _, high as _) == -1
        && io::Error::last_os_error().raw_os_error() == Some(libc::ENOSYS)
    {
        close_range_slow(fd_low, fd_high);
    }
}

#[cfg(target_os = "freebsd")]
fn close_range(low: ffi::c_uint, high: ffi::c_uint) -> ffi::c_int {
    unsafe { libc::close_range(low, high, 0) as ffi::c_int }
}

#[cfg(target_os = "linux")]
fn close_range(low: ffi::c_uint, high: ffi::c_uint) -> ffi::c_int {
    unsafe { libc::syscall(libc::SYS_close_range, low, high, 0) as ffi::c_int }
}

fn close_range_slow(low: Raw, high: Raw) {
    for fd in low..high {
        // The range can contain closed descriptors, so it cannot use Owned.
        let _ = _close(fd);
    }
}

pub fn ftruncate(fd: Borrowed<'_>, len: Offset) -> io::Result<()> {
    let ret = unsafe { suppress_iph!(c::ftruncate(fd.as_raw(), len)) };
    // On Windows, _chsize_s returns 0 on success, or a positive error code (errno value) on failure.
    // On other platforms, ftruncate returns 0 on success, or -1 on failure with errno set.
    cfg_select! {
        windows => {
            if ret != 0 {
                // _chsize_s returns errno directly; preserve it exactly.
                return Err(crate::os::io_error_from_errno(ret));
            }
        }
        _ => cvt(ret)?,
    };
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
        // _get_osfhandle is a CRT function that sets errno, not GetLastError()
        Err(crate::os::errno_io_error())
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
