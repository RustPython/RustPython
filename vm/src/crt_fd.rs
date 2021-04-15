//! A module implementing an io type backed by the C runtime's file descriptors, i.e. what's
//! returned from libc::open, even on windows.

use std::{cmp, ffi, fs, io, mem};

#[cfg(windows)]
use libc::commit as fsync;
#[cfg(windows)]
extern "C" {
    #[link_name = "_chsize_s"]
    fn ftruncate(fd: i32, len: i64) -> i32;
}
#[cfg(not(windows))]
use libc::{fsync, ftruncate};

#[inline]
fn cvt<T, I: num_traits::PrimInt>(ret: I, f: impl FnOnce(I) -> T) -> io::Result<T> {
    if ret < I::zero() {
        Err(io::Error::last_os_error())
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
        cvt(unsafe { libc::wopen(path.as_ptr(), flags, mode) }, Fd)
    }

    #[cfg(all(any(unix, target_os = "wasi"), not(target_os = "redox")))]
    pub fn openat(&self, path: &ffi::CStr, flags: i32, mode: i32) -> io::Result<Self> {
        cvt(
            unsafe { libc::openat(self.0, path.as_ptr(), flags, mode) },
            Fd,
        )
    }

    pub fn fsync(&self) -> io::Result<()> {
        cvt(unsafe { fsync(self.0) }, drop)
    }

    pub fn close(&self) -> io::Result<()> {
        cvt(unsafe { libc::close(self.0) }, drop)
    }

    pub fn ftruncate(&self, len: i64) -> io::Result<()> {
        cvt(unsafe { ftruncate(self.0, len) }, drop)
    }

    /// NOTE: it's not recommended to use ManuallyDrop::into_inner() to drop the file - it won't
    /// work on all platforms, and will swallow any errors you might want to handle.
    #[allow(unused)] // only used on windows atm
    pub(crate) fn as_rust_file(&self) -> io::Result<mem::ManuallyDrop<fs::File>> {
        #[cfg(windows)]
        let file = {
            use std::os::windows::io::FromRawHandle;
            let handle = self.to_raw_handle()?;
            unsafe { fs::File::from_raw_handle(handle) }
        };
        #[cfg(unix)]
        let file = {
            let fd = self.0;
            use std::os::unix::io::FromRawFd;
            if fd < 0 {
                return Err(io::Error::from_raw_os_error(libc::EBADF));
            }
            unsafe { fs::File::from_raw_fd(fd) }
        };
        #[cfg(target_os = "wasi")]
        let file = {
            let fd = self.0;
            if fd < 0 {
                return Err(io::Error::from_raw_os_error(libc::EBADF));
            }
            // SAFETY: as of now, File is a wrapper around WasiFd, which is a wrapper around
            // wasi::Fd (u32). This isn't likely to change, and if it does change to a different
            // sized integer, mem::transmute will fail.
            unsafe { mem::transmute::<u32, fs::File>(fd as u32) }
        };
        Ok(mem::ManuallyDrop::new(file))
    }

    #[cfg(windows)]
    pub(crate) fn to_raw_handle(&self) -> io::Result<std::os::windows::io::RawHandle> {
        use winapi::um::{handleapi::INVALID_HANDLE_VALUE, winnt::HANDLE};
        extern "C" {
            fn _get_osfhandle(fd: i32) -> libc::intptr_t;
        }
        let handle = unsafe { crate::suppress_iph!(_get_osfhandle(self.0)) } as HANDLE;
        if handle == INVALID_HANDLE_VALUE {
            Err(io::Error::last_os_error())
        } else {
            Ok(handle)
        }
    }
}

impl io::Write for &Fd {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let count = cmp::min(buf.len(), MAX_RW);
        cvt(
            unsafe { libc::write(self.0, buf.as_ptr() as _, count as _) },
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
            unsafe { libc::read(self.0, buf.as_mut_ptr() as _, count as _) },
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
