// spell-checker:disable
// TODO: we can move more os-specific bindings/interfaces from stdlib::{os, posix, nt} to here

use core::str::Utf8Error;
#[cfg(windows)]
use core::time::Duration;
use std::{
    env,
    ffi::{OsStr, OsString},
    io,
    path::PathBuf,
    process::ExitCode,
};
#[cfg(windows)]
use {
    crate::{crt_fd, fs},
    std::{os::windows::io::AsRawHandle, path::Path},
    windows_sys::Win32::{
        Foundation::FILETIME,
        Storage::FileSystem::{
            FILE_FLAG_BACKUP_SEMANTICS, INVALID_SET_FILE_POINTER, SetFilePointer, SetFileTime,
        },
    },
};

/// Convert exit code to std::process::ExitCode
///
/// On Windows, this supports the full u32 range including STATUS_CONTROL_C_EXIT (0xC000013A).
/// On other platforms, only the lower 8 bits are used.
#[must_use]
pub fn exit_code(code: u32) -> ExitCode {
    #[cfg(windows)]
    {
        // For large exit codes like STATUS_CONTROL_C_EXIT (0xC000013A),
        // we need to call std::process::exit() directly since ExitCode::from(u8)
        // would truncate the value, and ExitCode::from_raw() is still unstable.
        // FIXME: side effect in exit_code is not ideal.
        if code > u8::MAX as u32 {
            std::process::exit(code as i32)
        }
    }
    ExitCode::from(code as u8)
}

pub fn current_dir() -> io::Result<PathBuf> {
    env::current_dir()
}

#[must_use]
pub fn temp_dir() -> PathBuf {
    env::temp_dir()
}

pub fn var(key: &str) -> Result<String, env::VarError> {
    env::var(key)
}

pub fn var_os(key: impl AsRef<OsStr>) -> Option<OsString> {
    env::var_os(key)
}

#[must_use]
pub fn vars_os() -> env::VarsOs {
    env::vars_os()
}

#[must_use]
pub fn vars() -> env::Vars {
    env::vars()
}

/// # Safety
/// The caller must ensure no other threads can concurrently read or write
/// the process environment while this mutation is performed.
pub unsafe fn set_var(key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) {
    unsafe { env::set_var(key, value) };
}

/// # Safety
/// The caller must ensure no other threads can concurrently read or write
/// the process environment while this mutation is performed.
pub unsafe fn remove_var(key: impl AsRef<OsStr>) {
    unsafe { env::remove_var(key) };
}

pub fn set_current_dir(path: impl AsRef<std::path::Path>) -> io::Result<()> {
    env::set_current_dir(&path)?;

    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::System::Environment::SetEnvironmentVariableW;

        if let Ok(cwd) = env::current_dir() {
            let cwd_str = cwd.as_os_str();
            let mut cwd_wide: Vec<u16> = cwd_str.encode_wide().collect();

            let is_unc_like_path = cwd_wide.len() >= 2
                && ((cwd_wide[0] == b'\\' as u16 && cwd_wide[1] == b'\\' as u16)
                    || (cwd_wide[0] == b'/' as u16 && cwd_wide[1] == b'/' as u16));

            if !is_unc_like_path {
                let env_name: [u16; 4] = [b'=' as u16, cwd_wide[0], b':' as u16, 0];
                cwd_wide.push(0);
                unsafe {
                    SetEnvironmentVariableW(env_name.as_ptr(), cwd_wide.as_ptr());
                }
            }
        }
    }

    Ok(())
}

#[must_use]
pub fn process_id() -> u32 {
    std::process::id()
}

#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
pub fn cpu_count() -> usize {
    num_cpus::get()
}

#[cfg(not(any(not(target_arch = "wasm32"), target_os = "wasi")))]
pub fn cpu_count() -> usize {
    1
}

pub fn device_encoding(_fd: i32) -> Option<String> {
    #[cfg(any(target_os = "android", target_os = "redox"))]
    {
        return Some("UTF-8".to_owned());
    }

    #[cfg(all(target_arch = "wasm32", not(target_os = "wasi")))]
    {
        return Some("UTF-8".to_owned());
    }

    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Console;
        let cp = match _fd {
            0 => unsafe { Console::GetConsoleCP() },
            1 | 2 => unsafe { Console::GetConsoleOutputCP() },
            _ => 0,
        };

        Some(format!("cp{cp}"))
    }

    #[cfg(not(any(
        target_os = "android",
        target_os = "redox",
        windows,
        all(target_arch = "wasm32", not(target_os = "wasi"))
    )))]
    {
        let encoding = unsafe {
            let encoding = libc::nl_langinfo(libc::CODESET);
            if encoding.is_null() || encoding.read() == b'\0' as libc::c_char {
                "UTF-8".to_owned()
            } else {
                core::ffi::CStr::from_ptr(encoding)
                    .to_string_lossy()
                    .into_owned()
            }
        };

        Some(encoding)
    }
}

pub fn exit(code: i32) -> ! {
    std::process::exit(code)
}

pub fn rename(
    from: impl AsRef<std::path::Path>,
    to: impl AsRef<std::path::Path>,
) -> io::Result<()> {
    std::fs::rename(from, to)
}

#[cfg(windows)]
pub fn seek_fd(
    fd: crt_fd::Borrowed<'_>,
    position: crt_fd::Offset,
    how: i32,
) -> io::Result<crt_fd::Offset> {
    let handle = crt_fd::as_handle(fd)?;
    let mut distance_to_move: [i32; 2] = unsafe { core::mem::transmute(position) };
    let ret = unsafe {
        SetFilePointer(
            handle.as_raw_handle(),
            distance_to_move[0],
            &mut distance_to_move[1],
            how as _,
        )
    };
    if ret == INVALID_SET_FILE_POINTER {
        Err(io::Error::last_os_error())
    } else {
        distance_to_move[0] = ret as _;
        Ok(unsafe { core::mem::transmute::<[i32; 2], i64>(distance_to_move) })
    }
}

#[cfg(windows)]
fn filetime_from_duration(duration: Duration) -> FILETIME {
    let intervals = ((duration.as_secs() as i64 + 11644473600) * 10_000_000)
        + (duration.subsec_nanos() as i64 / 100);
    FILETIME {
        dwLowDateTime: intervals as u32,
        dwHighDateTime: (intervals >> 32) as u32,
    }
}

#[cfg(windows)]
pub fn set_file_times(
    path: impl AsRef<Path>,
    access: Duration,
    modified: Duration,
) -> io::Result<()> {
    let access = filetime_from_duration(access);
    let modified = filetime_from_duration(modified);
    let file = fs::open_write_with_custom_flags(path, FILE_FLAG_BACKUP_SEMANTICS)?;
    let ret = unsafe {
        SetFileTime(
            file.as_raw_handle() as _,
            core::ptr::null(),
            &access,
            &modified,
        )
    };
    if ret == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

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

/// Get the last error from C runtime library functions (like _dup, _dup2, _fstat, etc.)
/// CRT functions set errno, not GetLastError(), so we need to read errno directly.
#[cfg(windows)]
#[must_use]
pub fn errno_io_error() -> io::Error {
    let errno: i32 = get_errno();
    let winerror = errno_to_winerror(errno);
    io::Error::from_raw_os_error(winerror)
}

#[cfg(not(windows))]
#[must_use]
pub fn errno_io_error() -> io::Error {
    std::io::Error::last_os_error()
}

#[cfg(windows)]
pub fn get_errno() -> i32 {
    unsafe extern "C" {
        fn _get_errno(pValue: *mut i32) -> i32;
    }
    let mut errno = 0;
    unsafe { suppress_iph!(_get_errno(&mut errno)) };
    errno
}

#[cfg(not(windows))]
#[must_use]
pub fn get_errno() -> i32 {
    std::io::Error::last_os_error().posix_errno()
}

pub fn clear_errno() {
    set_errno(0);
}

/// Set errno to the specified value.
#[cfg(windows)]
pub fn set_errno(value: i32) {
    unsafe extern "C" {
        fn _set_errno(value: i32) -> i32;
    }
    unsafe { suppress_iph!(_set_errno(value)) };
}

#[cfg(unix)]
pub fn set_errno(value: i32) {
    nix::errno::Errno::from_raw(value).set();
}

#[cfg(unix)]
pub fn bytes_as_os_str(b: &[u8]) -> Result<&std::ffi::OsStr, Utf8Error> {
    use std::os::unix::ffi::OsStrExt;
    Ok(std::ffi::OsStr::from_bytes(b))
}

#[cfg(not(unix))]
pub fn bytes_as_os_str(b: &[u8]) -> Result<&std::ffi::OsStr, Utf8Error> {
    Ok(core::str::from_utf8(b)?.as_ref())
}

#[cfg(unix)]
pub use std::os::unix::ffi;

// WASIp1 uses stable std::os::wasi::ffi
#[cfg(all(target_os = "wasi", not(target_env = "p2")))]
pub use std::os::wasi::ffi;

// WASIp2: std::os::wasip2::ffi is unstable, so we provide a stable implementation
// leveraging WASI's UTF-8 string guarantee
#[cfg(all(target_os = "wasi", target_env = "p2"))]
pub mod ffi {
    use std::ffi::{OsStr, OsString};

    pub trait OsStrExt: sealed::Sealed {
        fn as_bytes(&self) -> &[u8];
        fn from_bytes(slice: &[u8]) -> &Self;
    }

    impl OsStrExt for OsStr {
        fn as_bytes(&self) -> &[u8] {
            // WASI strings are guaranteed to be UTF-8
            self.to_str().expect("wasip2 strings are UTF-8").as_bytes()
        }

        fn from_bytes(slice: &[u8]) -> &OsStr {
            // WASI strings are guaranteed to be UTF-8
            OsStr::new(core::str::from_utf8(slice).expect("wasip2 strings are UTF-8"))
        }
    }

    pub trait OsStringExt: sealed::Sealed {
        fn from_vec(vec: Vec<u8>) -> Self;
        fn into_vec(self) -> Vec<u8>;
    }

    impl OsStringExt for OsString {
        fn from_vec(vec: Vec<u8>) -> OsString {
            // WASI strings are guaranteed to be UTF-8
            OsString::from(String::from_utf8(vec).expect("wasip2 strings are UTF-8"))
        }

        fn into_vec(self) -> Vec<u8> {
            // WASI strings are guaranteed to be UTF-8
            self.to_str()
                .expect("wasip2 strings are UTF-8")
                .as_bytes()
                .to_vec()
        }
    }

    mod sealed {
        pub trait Sealed {}
        impl Sealed for std::ffi::OsStr {}
        impl Sealed for std::ffi::OsString {}
    }
}

#[cfg(windows)]
#[must_use]
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
#[must_use]
pub fn winerror_to_errno(winerror: i32) -> i32 {
    use libc::*;
    use windows_sys::Win32::{
        Foundation::*,
        Networking::WinSock::{
            WSAEACCES, WSAEBADF, WSAECONNABORTED, WSAECONNREFUSED, WSAECONNRESET, WSAEFAULT,
            WSAEINTR, WSAEINVAL, WSAEMFILE,
        },
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
        // Connection-related Windows error codes - map to Winsock error codes
        // which Python uses on Windows (errno.ECONNREFUSED = 10061, etc.)
        ERROR_CONNECTION_REFUSED => WSAECONNREFUSED,
        ERROR_CONNECTION_ABORTED => WSAECONNABORTED,
        ERROR_NETNAME_DELETED => WSAECONNRESET,
        ERROR_INVALID_FUNCTION
        | ERROR_INVALID_ACCESS
        | ERROR_INVALID_DATA
        | ERROR_INVALID_PARAMETER
        | ERROR_NEGATIVE_SEEK => EINVAL,
        _ => EINVAL,
    }
}
