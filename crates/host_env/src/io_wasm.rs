use core::ffi::CStr;
use std::io;

use crate::errno::errors::{EAGAIN, EBADF, EINTR, EINVAL, EISDIR};
pub use crate::file_mode::{
    FileMode, FileModeError, FileModeFlag, ParsedFileMode, parse_fileio_mode,
};
use crate::{crt_fd, wasi_p1};

#[derive(Clone, Copy, Debug)]
pub struct FileTargetInfo {
    pub blksize: Option<i64>,
}

pub fn inspect_file_target(fd: crt_fd::Borrowed<'_>) -> io::Result<FileTargetInfo> {
    let stat = wasi_p1::fdstat(fd.as_raw())?;
    if stat.fs_filetype == wasi_p1::FILETYPE_DIRECTORY {
        return Err(io::Error::from_raw_os_error(EISDIR));
    }
    Ok(FileTargetInfo { blksize: None })
}

pub fn open_path(path: &CStr, flags: i32, mode: i32) -> io::Result<crt_fd::Owned> {
    crt_fd::open(path, flags, mode)
}

pub fn should_forget_fd_after_inspect_error(err: &io::Error, fd_is_own: bool) -> bool {
    let errno = err.raw_os_error();
    (errno == Some(EISDIR) || errno == Some(EBADF)) && (!fd_is_own || errno == Some(EBADF))
}

pub fn seek_to_end(fd: crt_fd::Borrowed<'_>) -> io::Result<crt_fd::Offset> {
    seek(fd, 0, 2)
}

pub fn is_seekable(fd: crt_fd::Borrowed<'_>) -> bool {
    tell(fd).is_ok()
}

pub fn reads_without_waiting(fd: crt_fd::Borrowed<'_>) -> bool {
    is_seekable(fd)
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
    fd: crt_fd::Borrowed<'_>,
    offset: crt_fd::Offset,
    how: i32,
) -> io::Result<crt_fd::Offset> {
    if !validate_whence(how) {
        return Err(io::Error::from_raw_os_error(EINVAL));
    }
    wasi_p1::seek(fd.as_raw(), offset, how)
}

pub fn tell(fd: crt_fd::Borrowed<'_>) -> io::Result<crt_fd::Offset> {
    seek(fd, 0, 1)
}

pub fn isatty(fd: i32) -> bool {
    wasi_p1::is_tty(fd)
}

pub fn read_once(fd: crt_fd::Borrowed<'_>, buf: &mut [u8]) -> io::Result<usize> {
    crt_fd::read(fd, buf)
}

pub fn read_all(fd: crt_fd::Borrowed<'_>, out: &mut Vec<u8>) -> io::Result<()> {
    let mut buffer = [0; 8192];
    loop {
        match read_once(fd, &mut buffer) {
            Ok(0) => return Ok(()),
            Ok(n) => out.extend_from_slice(&buffer[..n]),
            Err(err) if is_interrupted_error(&err) => continue,
            Err(err) => return Err(err),
        }
    }
}

pub fn write_once(fd: crt_fd::Borrowed<'_>, buf: &[u8]) -> io::Result<usize> {
    crt_fd::write(fd, buf)
}

pub fn close_owned_fd(fd: crt_fd::Owned) -> io::Result<()> {
    crt_fd::close(fd)
}
