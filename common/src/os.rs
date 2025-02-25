// TODO: we can move more os-specific bindings/interfaces from stdlib::{os, posix, nt} to here

use std::{io, str::Utf8Error};

pub trait ErrorExt {
    fn posix_errno(&self) -> i32;
}

impl ErrorExt for io::Error {
    #[cfg(not(windows))]
    fn posix_errno(&self) -> i32 {
        self.raw_os_error().unwrap_or(0)
    }
    #[cfg(windows)]
    fn posix_errno(&self) -> i32 {
        let winerror = self.raw_os_error().unwrap_or(0);
        winerror_to_errno(winerror)
    }
}

#[cfg(windows)]
pub fn last_os_error() -> io::Error {
    let err = io::Error::last_os_error();
    // FIXME: probably not ideal, we need a bigger dichotomy between GetLastError and errno
    if err.raw_os_error() == Some(0) {
        unsafe extern "C" {
            fn _get_errno(pValue: *mut i32) -> i32;
        }
        let mut errno = 0;
        unsafe { suppress_iph!(_get_errno(&mut errno)) };
        let errno = errno_to_winerror(errno);
        io::Error::from_raw_os_error(errno)
    } else {
        err
    }
}

#[cfg(not(windows))]
pub fn last_os_error() -> io::Error {
    io::Error::last_os_error()
}

#[cfg(windows)]
pub fn last_posix_errno() -> i32 {
    let err = io::Error::last_os_error();
    if err.raw_os_error() == Some(0) {
        unsafe extern "C" {
            fn _get_errno(pValue: *mut i32) -> i32;
        }
        let mut errno = 0;
        unsafe { suppress_iph!(_get_errno(&mut errno)) };
        errno
    } else {
        err.posix_errno()
    }
}

#[cfg(not(windows))]
pub fn last_posix_errno() -> i32 {
    last_os_error().posix_errno()
}

#[cfg(unix)]
pub fn bytes_as_osstr(b: &[u8]) -> Result<&std::ffi::OsStr, Utf8Error> {
    use std::os::unix::ffi::OsStrExt;
    Ok(std::ffi::OsStr::from_bytes(b))
}

#[cfg(not(unix))]
pub fn bytes_as_osstr(b: &[u8]) -> Result<&std::ffi::OsStr, Utf8Error> {
    Ok(std::str::from_utf8(b)?.as_ref())
}

#[cfg(unix)]
pub use std::os::unix::ffi;
#[cfg(target_os = "wasi")]
pub use std::os::wasi::ffi;

#[cfg(windows)]
pub fn errno_to_winerror(errno: i32) -> i32 {
    use libc::*;
    use windows_sys::Win32::Foundation::*;
    let winerror = match errno {
        ENOENT => ERROR_FILE_NOT_FOUND,
        E2BIG => ERROR_BAD_ENVIRONMENT,
        ENOEXEC => ERROR_BAD_FORMAT,
        EBADF => ERROR_INVALID_HANDLE,
        ECHILD => ERROR_WAIT_NO_CHILDREN,
        EAGAIN => ERROR_NO_PROC_SLOTS,
        ENOMEM => ERROR_NOT_ENOUGH_MEMORY,
        EACCES => ERROR_ACCESS_DENIED,
        EEXIST => ERROR_FILE_EXISTS,
        EXDEV => ERROR_NOT_SAME_DEVICE,
        ENOTDIR => ERROR_DIRECTORY,
        EMFILE => ERROR_TOO_MANY_OPEN_FILES,
        ENOSPC => ERROR_DISK_FULL,
        EPIPE => ERROR_BROKEN_PIPE,
        ENOTEMPTY => ERROR_DIR_NOT_EMPTY,
        EILSEQ => ERROR_NO_UNICODE_TRANSLATION,
        EINVAL => ERROR_INVALID_FUNCTION,
        _ => ERROR_INVALID_FUNCTION,
    };
    winerror as _
}

// winerror: https://learn.microsoft.com/windows/win32/debug/system-error-codes--0-499-
// errno: https://learn.microsoft.com/cpp/c-runtime-library/errno-constants?view=msvc-170
#[cfg(windows)]
pub fn winerror_to_errno(winerror: i32) -> i32 {
    use libc::*;
    use windows_sys::Win32::{
        Foundation::*,
        Networking::WinSock::{WSAEACCES, WSAEBADF, WSAEFAULT, WSAEINTR, WSAEINVAL, WSAEMFILE},
    };
    // Unwrap FACILITY_WIN32 HRESULT errors.
    // if ((winerror & 0xFFFF0000) == 0x80070000) {
    //     winerror &= 0x0000FFFF;
    // }

    // Winsock error codes (10000-11999) are errno values.
    if (10000..12000).contains(&winerror) {
        match winerror {
            WSAEINTR | WSAEBADF | WSAEACCES | WSAEFAULT | WSAEINVAL | WSAEMFILE => {
                // Winsock definitions of errno values. See WinSock2.h
                return winerror - 10000;
            }
            _ => return winerror as _,
        }
    }

    #[allow(non_upper_case_globals)]
    match winerror as u32 {
        ERROR_FILE_NOT_FOUND
        | ERROR_PATH_NOT_FOUND
        | ERROR_INVALID_DRIVE
        | ERROR_NO_MORE_FILES
        | ERROR_BAD_NETPATH
        | ERROR_BAD_NET_NAME
        | ERROR_BAD_PATHNAME
        | ERROR_FILENAME_EXCED_RANGE => ENOENT,
        ERROR_BAD_ENVIRONMENT => E2BIG,
        ERROR_BAD_FORMAT
        | ERROR_INVALID_STARTING_CODESEG
        | ERROR_INVALID_STACKSEG
        | ERROR_INVALID_MODULETYPE
        | ERROR_INVALID_EXE_SIGNATURE
        | ERROR_EXE_MARKED_INVALID
        | ERROR_BAD_EXE_FORMAT
        | ERROR_ITERATED_DATA_EXCEEDS_64k
        | ERROR_INVALID_MINALLOCSIZE
        | ERROR_DYNLINK_FROM_INVALID_RING
        | ERROR_IOPL_NOT_ENABLED
        | ERROR_INVALID_SEGDPL
        | ERROR_AUTODATASEG_EXCEEDS_64k
        | ERROR_RING2SEG_MUST_BE_MOVABLE
        | ERROR_RELOC_CHAIN_XEEDS_SEGLIM
        | ERROR_INFLOOP_IN_RELOC_CHAIN => ENOEXEC,
        ERROR_INVALID_HANDLE | ERROR_INVALID_TARGET_HANDLE | ERROR_DIRECT_ACCESS_HANDLE => EBADF,
        ERROR_WAIT_NO_CHILDREN | ERROR_CHILD_NOT_COMPLETE => ECHILD,
        ERROR_NO_PROC_SLOTS | ERROR_MAX_THRDS_REACHED | ERROR_NESTING_NOT_ALLOWED => EAGAIN,
        ERROR_ARENA_TRASHED
        | ERROR_NOT_ENOUGH_MEMORY
        | ERROR_INVALID_BLOCK
        | ERROR_NOT_ENOUGH_QUOTA => ENOMEM,
        ERROR_ACCESS_DENIED
        | ERROR_CURRENT_DIRECTORY
        | ERROR_WRITE_PROTECT
        | ERROR_BAD_UNIT
        | ERROR_NOT_READY
        | ERROR_BAD_COMMAND
        | ERROR_CRC
        | ERROR_BAD_LENGTH
        | ERROR_SEEK
        | ERROR_NOT_DOS_DISK
        | ERROR_SECTOR_NOT_FOUND
        | ERROR_OUT_OF_PAPER
        | ERROR_WRITE_FAULT
        | ERROR_READ_FAULT
        | ERROR_GEN_FAILURE
        | ERROR_SHARING_VIOLATION
        | ERROR_LOCK_VIOLATION
        | ERROR_WRONG_DISK
        | ERROR_SHARING_BUFFER_EXCEEDED
        | ERROR_NETWORK_ACCESS_DENIED
        | ERROR_CANNOT_MAKE
        | ERROR_FAIL_I24
        | ERROR_DRIVE_LOCKED
        | ERROR_SEEK_ON_DEVICE
        | ERROR_NOT_LOCKED
        | ERROR_LOCK_FAILED
        | 35 => EACCES,
        ERROR_FILE_EXISTS | ERROR_ALREADY_EXISTS => EEXIST,
        ERROR_NOT_SAME_DEVICE => EXDEV,
        ERROR_DIRECTORY => ENOTDIR,
        ERROR_TOO_MANY_OPEN_FILES => EMFILE,
        ERROR_DISK_FULL => ENOSPC,
        ERROR_BROKEN_PIPE | ERROR_NO_DATA => EPIPE,
        ERROR_DIR_NOT_EMPTY => ENOTEMPTY,
        ERROR_NO_UNICODE_TRANSLATION => EILSEQ,
        ERROR_INVALID_FUNCTION
        | ERROR_INVALID_ACCESS
        | ERROR_INVALID_DATA
        | ERROR_INVALID_PARAMETER
        | ERROR_NEGATIVE_SEEK => EINVAL,
        _ => EINVAL,
    }
}
