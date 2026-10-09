use alloc::ffi::CString;
use core::ffi::CStr;
use rustix::fd::AsFd;
use std::{ffi::OsStr, io, path::Path};

pub use super::posix_unix_like::*;

use crate::{crt_fd, os::CheckLibcResult};

pub fn stat_fd(fd: crate::crt_fd::Borrowed<'_>) -> io::Result<crate::fileutils::StatStruct> {
    crate::fileutils::fstat(fd)
}

#[allow(
    clippy::useless_conversion,
    reason = "time_t and long widths differ across targets"
)]
pub fn set_file_times_at(
    dir_fd: i32,
    path: &CStr,
    access: crate::os::FileTime,
    modified: crate::os::FileTime,
    follow_symlinks: bool,
) -> io::Result<()> {
    let ts = |time: crate::os::FileTime| -> io::Result<libc::timespec> {
        Ok(libc::timespec {
            tv_sec: time
                .seconds
                .try_into()
                .map_err(|_| io::Error::from_raw_os_error(libc::EOVERFLOW))?,
            tv_nsec: time
                .nanoseconds
                .try_into()
                .map_err(|_| io::Error::from_raw_os_error(libc::EOVERFLOW))?,
        })
    };
    let times = [ts(access)?, ts(modified)?];
    unsafe {
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
    }
    .check_libc_neg()?;
    Ok(())
}
