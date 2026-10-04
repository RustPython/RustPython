use core::ffi::CStr;
use std::io;

use crate::crt_fd;
use crate::errno::errors::{EAGAIN, EBADF, EINTR, EISDIR};
use crate::os::{O_APPEND, O_CREAT, O_EXCL, O_RDONLY, O_RDWR, O_TRUNC, O_WRONLY};

bitflagset::bitflag! {
    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    #[repr(u8)]
    pub enum FileModeFlag {
        Created = 0,
        Readable = 1,
        Writable = 2,
        Appending = 3,
    }
}

bitflagset::bitflagset! {
    #[derive(Copy, Clone, PartialEq, Eq)]
    pub struct FileMode(u8): FileModeFlag
}

impl FileMode {
    pub const CREATED: Self = Self::from_element(FileModeFlag::Created);
    pub const READABLE: Self = Self::from_element(FileModeFlag::Readable);
    pub const WRITABLE: Self = Self::from_element(FileModeFlag::Writable);
    pub const APPENDING: Self = Self::from_element(FileModeFlag::Appending);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileModeError {
    Invalid,
    BadRwa,
}

impl FileModeError {
    pub fn error_msg(self, mode_str: &str) -> String {
        match self {
            Self::Invalid => format!("invalid mode: {mode_str}"),
            Self::BadRwa => {
                "Must have exactly one of create/read/write/append mode and at most one plus"
                    .to_owned()
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ParsedFileMode {
    pub mode: FileMode,
    pub flags: i32,
}

impl FileMode {
    pub const fn raw_mode(self) -> &'static str {
        if self.contains(&FileModeFlag::Created) {
            if self.contains(&FileModeFlag::Readable) {
                "xb+"
            } else {
                "xb"
            }
        } else if self.contains(&FileModeFlag::Appending) {
            if self.contains(&FileModeFlag::Readable) {
                "ab+"
            } else {
                "ab"
            }
        } else if self.contains(&FileModeFlag::Readable) {
            if self.contains(&FileModeFlag::Writable) {
                "rb+"
            } else {
                "rb"
            }
        } else {
            "wb"
        }
    }
}

pub fn parse_fileio_mode(mode_str: &str) -> Result<ParsedFileMode, FileModeError> {
    let mut flags = 0;
    let mut plus = false;
    let mut binary = false;
    let mut rwa = false;
    let mut mode = FileMode::empty();
    for c in mode_str.bytes() {
        match c {
            b'x' => {
                if rwa {
                    return Err(FileModeError::BadRwa);
                }
                rwa = true;
                mode |= FileMode::WRITABLE | FileMode::CREATED;
                flags |= O_EXCL | O_CREAT;
            }
            b'r' => {
                if rwa {
                    return Err(FileModeError::BadRwa);
                }
                rwa = true;
                mode |= FileMode::READABLE;
            }
            b'w' => {
                if rwa {
                    return Err(FileModeError::BadRwa);
                }
                rwa = true;
                mode |= FileMode::WRITABLE;
                flags |= O_CREAT | O_TRUNC;
            }
            b'a' => {
                if rwa {
                    return Err(FileModeError::BadRwa);
                }
                rwa = true;
                mode |= FileMode::WRITABLE | FileMode::APPENDING;
                flags |= O_APPEND | O_CREAT;
            }
            b'+' => {
                if plus {
                    return Err(FileModeError::BadRwa);
                }
                plus = true;
                mode |= FileMode::READABLE | FileMode::WRITABLE;
            }
            b'b' => {
                if binary {
                    return Err(FileModeError::Invalid);
                }
                binary = true;
            }
            _ => return Err(FileModeError::Invalid),
        }
    }

    if !rwa {
        return Err(FileModeError::BadRwa);
    }

    if mode.is_superset(&(FileMode::READABLE | FileMode::WRITABLE)) {
        flags |= O_RDWR;
    } else if mode.contains(&FileModeFlag::Readable) {
        flags |= O_RDONLY;
    } else {
        flags |= O_WRONLY;
    }

    Ok(ParsedFileMode { mode, flags })
}

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
