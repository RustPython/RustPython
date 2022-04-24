//! A module implementing an io type backed by the C runtime's file descriptors, i.e. what's
//! returned from libc::open, even on windows.

use std::{cmp, ffi, io};

#[cfg(windows)]
use libc::commit as fsync;
#[cfg(windows)]
extern "C" {
    #[link_name = "_chsize_s"]
    fn ftruncate(fd: i32, len: i64) -> i32;
}
#[cfg(not(windows))]
use libc::{fsync, ftruncate};

// this is basically what CPython has for Py_off_t; windows uses long long
// for offsets, other platforms just use off_t
#[cfg(not(windows))]
pub type Offset = libc::off_t;
#[cfg(windows)]
pub type Offset = libc::c_longlong;

// copied from stdlib::os
#[cfg(windows)]
fn errno() -> io::Error {
    let err = io::Error::last_os_error();
    // FIXME: probably not ideal, we need a bigger dichotomy between GetLastError and errno
    if err.raw_os_error() == Some(0) {
        extern "C" {
            fn _get_errno(pValue: *mut i32) -> i32;
        }
        let mut e = 0;
        unsafe { suppress_iph!(_get_errno(&mut e)) };
        io::Error::from_raw_os_error(e)
    } else {
        err
    }
}
#[cfg(not(windows))]
fn errno() -> io::Error {
    io::Error::last_os_error()
}

#[inline]
fn cvt<T, I: num_traits::PrimInt>(ret: I, f: impl FnOnce(I) -> T) -> io::Result<T> {
    if ret < I::zero() {
        Err(errno())
    } else {
        Ok(f(ret))
    }
}

const MAX_RW: usize = if cfg!(any(windows, target_vendor = "apple")) {
    i32::MAX as usize
} else {
    isize::MAX as usize
};

#[derive(Copy, Clone, PartialEq, Eq)]
#[repr(transparent)]
pub struct Fd(pub i32);

impl Fd {
    pub fn open(path: &ffi::CStr, flags: i32, mode: i32) -> io::Result<Self> {
        cvt(unsafe { libc::open(path.as_ptr(), flags, mode) }, Fd)
    }

    #[cfg(windows)]
    pub fn wopen(path: &widestring::WideCStr, flags: i32, mode: i32) -> io::Result<Self> {
        cvt(
            unsafe { suppress_iph!(libc::wopen(path.as_ptr(), flags, mode)) },
            Fd,
        )
    }

    #[cfg(all(any(unix, target_os = "wasi"), not(target_os = "redox")))]
    pub fn openat(&self, path: &ffi::CStr, flags: i32, mode: i32) -> io::Result<Self> {
        cvt(
            unsafe { libc::openat(self.0, path.as_ptr(), flags, mode) },
            Fd,
        )
    }

    pub fn fsync(&self) -> io::Result<()> {
        cvt(unsafe { suppress_iph!(fsync(self.0)) }, drop)
    }

    pub fn close(&self) -> io::Result<()> {
        cvt(unsafe { suppress_iph!(libc::close(self.0)) }, drop)
    }

    pub fn ftruncate(&self, len: Offset) -> io::Result<()> {
        cvt(unsafe { suppress_iph!(ftruncate(self.0, len)) }, drop)
    }

    #[cfg(windows)]
    pub fn to_raw_handle(&self) -> io::Result<std::os::windows::io::RawHandle> {
        extern "C" {
            fn _get_osfhandle(fd: i32) -> libc::intptr_t;
        }
        let handle = unsafe { suppress_iph!(_get_osfhandle(self.0)) };
        if handle == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(handle as _)
        }
    }
}

impl io::Write for &Fd {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let count = cmp::min(buf.len(), MAX_RW);
        cvt(
            unsafe { suppress_iph!(libc::write(self.0, buf.as_ptr() as _, count as _)) },
            |i| i as usize,
        )
    }

    #[inline]
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl io::Write for Fd {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        (&*self).write(buf)
    }

    #[inline]
    fn flush(&mut self) -> io::Result<()> {
        (&*self).flush()
    }
}

impl io::Read for &Fd {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let count = cmp::min(buf.len(), MAX_RW);
        cvt(
            unsafe { suppress_iph!(libc::read(self.0, buf.as_mut_ptr() as _, count as _)) },
            |i| i as usize,
        )
    }
}

impl io::Read for Fd {
    #[inline]
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        (&*self).read(buf)
    }
}
