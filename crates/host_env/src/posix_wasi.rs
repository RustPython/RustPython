use alloc::ffi::CString;
use core::{ffi::CStr, time::Duration};
use std::{ffi::OsStr, io};

pub fn make_dir(path: &CStr, mode: u32) -> io::Result<()> {
    let ret = unsafe { libc::mkdir(path.as_ptr(), mode as _) };
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub fn make_dir_at(dir_fd: i32, path: &CStr, mode: u32) -> io::Result<()> {
    let ret = unsafe { libc::mkdirat(dir_fd, path.as_ptr(), mode as _) };
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub fn remove_dir_at(dir_fd: i32, path: &CStr) -> io::Result<()> {
    let ret = unsafe { libc::unlinkat(dir_fd, path.as_ptr(), libc::AT_REMOVEDIR) };
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub fn stat_path(
    path: &OsStr,
    dir_fd: Option<i32>,
    follow_symlinks: bool,
) -> io::Result<Option<crate::fileutils::StatStruct>> {
    use crate::os::ffi::OsStrExt;

    let path = match CString::new(path.as_bytes()) {
        Ok(path) => path,
        Err(_) => return Err(io::Error::from(io::ErrorKind::InvalidInput)),
    };

    let mut stat = core::mem::MaybeUninit::uninit();
    if let Some(dir_fd) = dir_fd {
        let flags = if follow_symlinks {
            0
        } else {
            libc::AT_SYMLINK_NOFOLLOW
        };
        let ret = unsafe { libc::fstatat(dir_fd, path.as_ptr(), stat.as_mut_ptr(), flags) };
        if ret < 0 {
            return Err(io::Error::last_os_error());
        }
        return Ok(Some(unsafe { stat.assume_init() }));
    }

    let ret = if follow_symlinks {
        unsafe { libc::stat(path.as_ptr(), stat.as_mut_ptr()) }
    } else {
        unsafe { libc::lstat(path.as_ptr(), stat.as_mut_ptr()) }
    };
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(Some(unsafe { stat.assume_init() }))
    }
}

pub fn stat_fd(fd: crate::crt_fd::Borrowed<'_>) -> io::Result<crate::fileutils::StatStruct> {
    crate::fileutils::fstat(fd)
}

pub fn set_file_times_at(
    dir_fd: i32,
    path: &CStr,
    access: Duration,
    modified: Duration,
    follow_symlinks: bool,
) -> io::Result<()> {
    let ts = |d: Duration| libc::timespec {
        tv_sec: d.as_secs() as _,
        tv_nsec: d.subsec_nanos() as _,
    };
    let times = [ts(access), ts(modified)];
    let ret = unsafe {
        libc::utimensat(
            dir_fd,
            path.as_ptr(),
            times.as_ptr(),
            if follow_symlinks {
                0
            } else {
                libc::AT_SYMLINK_NOFOLLOW
            },
        )
    };
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
