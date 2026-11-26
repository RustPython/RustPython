// spell-checker:disable

use core::ffi::CStr;
use std::io;

// Linux limits from <linux/limits.h>.
pub const XATTR_SIZE_MAX: usize = 65_536;
const XATTR_LIST_MAX: usize = 65_536;

/// Paths use a C string; a Python descriptor is deliberately an unchecked
/// integer so EBADF is reported by the OS without inventing a BorrowedFd.
pub enum Target<'path> {
    Follow(&'path CStr),
    NoFollow(&'path CStr),
    Fd(i32),
}

impl Target<'_> {
    pub fn get(&self, name: &CStr) -> io::Result<Vec<u8>> {
        read::<128, XATTR_SIZE_MAX>(|buf| {
            let ptr = buf.as_mut_ptr().cast();
            let len = buf.len();
            // SAFETY: paths and name are NUL-terminated; buf is valid for len writes.
            unsafe {
                match self {
                    Self::Follow(path) => libc::getxattr(path.as_ptr(), name.as_ptr(), ptr, len),
                    Self::NoFollow(path) => libc::lgetxattr(path.as_ptr(), name.as_ptr(), ptr, len),
                    Self::Fd(fd) => libc::fgetxattr(*fd, name.as_ptr(), ptr, len),
                }
            }
        })
    }

    pub fn list(&self) -> io::Result<Vec<u8>> {
        read::<256, XATTR_LIST_MAX>(|buf| {
            let ptr = buf.as_mut_ptr().cast();
            let len = buf.len();
            // SAFETY: paths are NUL-terminated; buf is valid for len writes.
            unsafe {
                match self {
                    Self::Follow(path) => libc::listxattr(path.as_ptr(), ptr, len),
                    Self::NoFollow(path) => libc::llistxattr(path.as_ptr(), ptr, len),
                    Self::Fd(fd) => libc::flistxattr(*fd, ptr, len),
                }
            }
        })
    }

    pub fn set(&self, name: &CStr, value: &[u8], flags: i32) -> io::Result<()> {
        let ptr = value.as_ptr().cast();
        let len = value.len();
        // SAFETY: paths and name are NUL-terminated; value is valid for len reads.
        let result = unsafe {
            match self {
                Self::Follow(path) => libc::setxattr(path.as_ptr(), name.as_ptr(), ptr, len, flags),
                Self::NoFollow(path) => {
                    libc::lsetxattr(path.as_ptr(), name.as_ptr(), ptr, len, flags)
                }
                Self::Fd(fd) => libc::fsetxattr(*fd, name.as_ptr(), ptr, len, flags),
            }
        };
        cvt(result as isize).map(drop)
    }

    pub fn remove(&self, name: &CStr) -> io::Result<()> {
        // SAFETY: paths and name are NUL-terminated; descriptors remain integers.
        let result = unsafe {
            match self {
                Self::Follow(path) => libc::removexattr(path.as_ptr(), name.as_ptr()),
                Self::NoFollow(path) => libc::lremovexattr(path.as_ptr(), name.as_ptr()),
                Self::Fd(fd) => libc::fremovexattr(*fd, name.as_ptr()),
            }
        };
        cvt(result as isize).map(drop)
    }
}

fn cvt(result: isize) -> io::Result<usize> {
    usize::try_from(result).map_err(|_| io::Error::last_os_error())
}

// Match CPython's bounded two attempts, without a size-query race.
fn read<const SMALL: usize, const MAX: usize>(
    mut call: impl FnMut(&mut [u8]) -> isize,
) -> io::Result<Vec<u8>> {
    let mut small = [0; SMALL];
    match cvt(call(&mut small)) {
        Ok(size) => {
            let mut out = Vec::new();
            out.try_reserve_exact(size)
                .map_err(|_| io::ErrorKind::OutOfMemory)?;
            out.extend_from_slice(&small[..size]);
            Ok(out)
        }
        Err(err) if err.raw_os_error() == Some(libc::ERANGE) => {
            let mut out = Vec::new();
            out.try_reserve_exact(MAX)
                .map_err(|_| io::ErrorKind::OutOfMemory)?;
            out.resize(MAX, 0);
            let size = cvt(call(&mut out))?;
            out.truncate(size);
            Ok(out)
        }
        Err(err) => Err(err),
    }
}
