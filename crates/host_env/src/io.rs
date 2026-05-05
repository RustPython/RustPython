#[cfg(any(unix, target_os = "wasi"))]
use core::ffi::CStr;
use std::io;

#[cfg(any(unix, target_os = "wasi"))]
use crate::fileutils;
use crate::{crt_fd, os};

bitflags::bitflags! {
    #[derive(Copy, Clone, Debug, PartialEq)]
    pub struct FileMode: u8 {
        const CREATED   = 0b0001;
        const READABLE  = 0b0010;
        const WRITABLE  = 0b0100;
        const APPENDING = 0b1000;
    }
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
        if self.contains(Self::CREATED) {
            if self.contains(Self::READABLE) {
                "xb+"
            } else {
                "xb"
            }
        } else if self.contains(Self::APPENDING) {
            if self.contains(Self::READABLE) {
                "ab+"
            } else {
                "ab"
            }
        } else if self.contains(Self::READABLE) {
            if self.contains(Self::WRITABLE) {
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
    let mut rwa = false;
    let mut mode = FileMode::empty();
    for c in mode_str.bytes() {
        match c {
            b'x' => {
                if rwa {
                    return Err(FileModeError::BadRwa);
                }
                rwa = true;
                mode.insert(FileMode::WRITABLE | FileMode::CREATED);
                flags |= libc::O_EXCL | libc::O_CREAT;
            }
            b'r' => {
                if rwa {
                    return Err(FileModeError::BadRwa);
                }
                rwa = true;
                mode.insert(FileMode::READABLE);
            }
            b'w' => {
                if rwa {
                    return Err(FileModeError::BadRwa);
                }
                rwa = true;
                mode.insert(FileMode::WRITABLE);
                flags |= libc::O_CREAT | libc::O_TRUNC;
            }
            b'a' => {
                if rwa {
                    return Err(FileModeError::BadRwa);
                }
                rwa = true;
                mode.insert(FileMode::WRITABLE | FileMode::APPENDING);
                flags |= libc::O_APPEND | libc::O_CREAT;
            }
            b'+' => {
                if plus {
                    return Err(FileModeError::BadRwa);
                }
                plus = true;
                mode.insert(FileMode::READABLE | FileMode::WRITABLE);
            }
            b'b' => {}
            _ => return Err(FileModeError::Invalid),
        }
    }

    if !rwa {
        return Err(FileModeError::BadRwa);
    }

    if mode.contains(FileMode::READABLE | FileMode::WRITABLE) {
        flags |= libc::O_RDWR;
    } else if mode.contains(FileMode::READABLE) {
        flags |= libc::O_RDONLY;
    } else {
        flags |= libc::O_WRONLY;
    }

    #[cfg(windows)]
    {
        flags |= libc::O_BINARY | libc::O_NOINHERIT;
    }
    #[cfg(unix)]
    {
        flags |= libc::O_CLOEXEC;
    }

    Ok(ParsedFileMode { mode, flags })
}

#[derive(Clone, Copy, Debug)]
pub struct FileTargetInfo {
    pub blksize: Option<i64>,
}

#[cfg(any(unix, target_os = "wasi"))]
pub fn inspect_file_target(fd: crt_fd::Borrowed<'_>) -> io::Result<FileTargetInfo> {
    let status = fileutils::fstat(fd)?;
    if (status.st_mode & libc::S_IFMT) == libc::S_IFDIR {
        return Err(io::Error::from_raw_os_error(libc::EISDIR));
    }
    #[allow(clippy::useless_conversion, reason = "needed for 32-bit platforms")]
    let blksize = (status.st_blksize > 1).then(|| i64::from(status.st_blksize));
    Ok(FileTargetInfo { blksize })
}

#[cfg(windows)]
pub fn inspect_file_target(fd: crt_fd::Borrowed<'_>) -> io::Result<FileTargetInfo> {
    if !crate::nt::fd_exists(fd) {
        return Err(io::Error::from_raw_os_error(
            crate::nt::ERROR_INVALID_HANDLE_I32,
        ));
    }
    Ok(FileTargetInfo { blksize: None })
}

#[cfg(any(unix, target_os = "wasi"))]
pub fn open_path(path: &CStr, flags: i32, mode: i32) -> io::Result<crt_fd::Owned> {
    crt_fd::open(path, flags, mode)
}

#[cfg(windows)]
pub fn open_path(path: &widestring::WideCStr, flags: i32, mode: i32) -> io::Result<crt_fd::Owned> {
    crt_fd::wopen(path, flags, mode)
}

#[cfg(windows)]
pub fn should_forget_fd_after_inspect_error(err: &io::Error, _fd_is_own: bool) -> bool {
    err.raw_os_error() == Some(crate::nt::ERROR_INVALID_HANDLE_I32)
}

#[cfg(any(unix, target_os = "wasi"))]
pub fn should_forget_fd_after_inspect_error(err: &io::Error, fd_is_own: bool) -> bool {
    let errno = err.raw_os_error();
    (errno == Some(libc::EISDIR) || errno == Some(libc::EBADF))
        && (!fd_is_own || errno == Some(libc::EBADF))
}

pub fn seek_to_end(fd: crt_fd::Borrowed<'_>) -> io::Result<crt_fd::Offset> {
    os::seek_fd(fd, 0, libc::SEEK_END)
}

pub fn is_seekable(fd: crt_fd::Borrowed<'_>) -> bool {
    os::seek_fd(fd, 0, libc::SEEK_CUR).is_ok()
}

pub fn validate_whence(whence: i32) -> bool {
    let standard = (0..=2).contains(&whence);
    #[cfg(any(target_os = "dragonfly", target_os = "freebsd", target_os = "linux"))]
    {
        standard || matches!(whence, libc::SEEK_DATA | libc::SEEK_HOLE)
    }
    #[cfg(not(any(target_os = "dragonfly", target_os = "freebsd", target_os = "linux")))]
    {
        standard
    }
}

pub fn is_interrupted_errno(errno: i32) -> bool {
    errno == libc::EINTR
}

pub fn is_interrupted_error(err: &io::Error) -> bool {
    err.raw_os_error() == Some(libc::EINTR)
}

pub fn is_would_block_error(err: &io::Error) -> bool {
    err.kind() == io::ErrorKind::WouldBlock || err.raw_os_error() == Some(libc::EAGAIN)
}

pub fn seek(
    fd: crt_fd::Borrowed<'_>,
    offset: crt_fd::Offset,
    how: i32,
) -> io::Result<crt_fd::Offset> {
    os::seek_fd(fd, offset, how)
}

pub fn tell(fd: crt_fd::Borrowed<'_>) -> io::Result<crt_fd::Offset> {
    os::seek_fd(fd, 0, libc::SEEK_CUR)
}

pub fn isatty(fd: i32) -> bool {
    os::isatty(fd)
}

pub fn read_once(fd: crt_fd::Borrowed<'_>, buf: &mut [u8]) -> io::Result<usize> {
    crt_fd::read(fd, buf)
}

pub fn read_all(fd: crt_fd::Borrowed<'_>, out: &mut Vec<u8>) -> io::Result<()> {
    let mut fd = fd;
    std::io::Read::read_to_end(&mut fd, out).map(|_| ())
}

pub fn write_once(fd: crt_fd::Borrowed<'_>, buf: &[u8]) -> io::Result<usize> {
    crt_fd::write(fd, buf)
}

pub fn close_owned_fd(fd: crt_fd::Owned) -> io::Result<()> {
    crt_fd::close(fd)
}
