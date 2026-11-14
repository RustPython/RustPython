use core::ffi::CStr;
use std::io;

use crate::crt_fd;
use crate::errno::errors::{EAGAIN, EBADF, EINTR, EISDIR};
pub use crate::file_mode::{
    FileMode, FileModeError, FileModeFlag, ParsedFileMode, parse_fileio_mode,
};

#[derive(Clone, Copy, Debug)]
pub struct FileTargetInfo {
    pub blksize: Option<i64>,
}

pub fn inspect_file_target(_fd: crt_fd::Borrowed<'_>) -> io::Result<FileTargetInfo> {
    Err(io::Error::from_raw_os_error(EBADF))
}

pub fn open_path(_path: &CStr, _flags: i32, _mode: i32) -> io::Result<crt_fd::Owned> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "host filesystem is unsupported on this platform",
    ))
}

pub fn should_forget_fd_after_inspect_error(err: &io::Error, fd_is_own: bool) -> bool {
    let errno = err.raw_os_error();
    (errno == Some(EISDIR) || errno == Some(EBADF)) && (!fd_is_own || errno == Some(EBADF))
}

pub fn seek_to_end(_fd: crt_fd::Borrowed<'_>) -> io::Result<crt_fd::Offset> {
    Err(io::Error::from_raw_os_error(EBADF))
}

pub fn is_seekable(_fd: crt_fd::Borrowed<'_>) -> bool {
    false
}

pub fn reads_without_waiting(_fd: crt_fd::Borrowed<'_>) -> bool {
    false
}

pub fn validate_whence(whence: i32) -> bool {
    (0..=2).contains(&whence)
}

pub fn is_interrupted_errno(errno: i32) -> bool {
    errno == EINTR
}

pub fn is_interrupted_error(err: &io::Error) -> bool {
    err.raw_os_error() == Some(EINTR)
}

pub fn is_would_block_error(err: &io::Error) -> bool {
    err.raw_os_error() == Some(EAGAIN)
}

pub fn seek(
    _fd: crt_fd::Borrowed<'_>,
    _offset: crt_fd::Offset,
    _how: i32,
) -> io::Result<crt_fd::Offset> {
    Err(io::Error::from_raw_os_error(EBADF))
}

pub fn tell(_fd: crt_fd::Borrowed<'_>) -> io::Result<crt_fd::Offset> {
    Err(io::Error::from_raw_os_error(EBADF))
}

pub fn isatty(_fd: i32) -> bool {
    false
}

pub fn read_once(_fd: crt_fd::Borrowed<'_>, _buf: &mut [u8]) -> io::Result<usize> {
    Err(io::Error::from_raw_os_error(EBADF))
}

pub fn read_all(_fd: crt_fd::Borrowed<'_>, _out: &mut Vec<u8>) -> io::Result<()> {
    Err(io::Error::from_raw_os_error(EBADF))
}

pub fn write_once(_fd: crt_fd::Borrowed<'_>, _buf: &[u8]) -> io::Result<usize> {
    Err(io::Error::from_raw_os_error(EBADF))
}

pub fn close_owned_fd(_fd: crt_fd::Owned) -> io::Result<()> {
    Err(io::Error::from_raw_os_error(EBADF))
}
