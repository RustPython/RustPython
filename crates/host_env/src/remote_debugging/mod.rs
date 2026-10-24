//! Read-only discovery of CPython's exported runtime metadata.
//!
//! A successful cookie probe identifies a runtime; it does not validate its
//! debug offsets or promise that stack unwinding is supported.

#[cfg(all(target_os = "linux", not(target_env = "uclibc")))]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(any(
    all(target_os = "linux", not(target_env = "uclibc")),
    target_os = "macos",
    windows
))]
mod object_file;

#[cfg(any(
    all(target_os = "linux", not(target_env = "uclibc")),
    target_os = "macos",
    windows
))]
mod common;

#[cfg(all(target_os = "linux", not(target_env = "uclibc")))]
use common::COOKIE;
#[cfg(any(
    all(target_os = "linux", not(target_env = "uclibc")),
    target_os = "macos",
    windows
))]
use common::{ReadBudget, RemoteRead, has_cookie, invalid_data, stops_probe};

pub const PROCESS_VM_READV_SUPPORTED: u8 =
    cfg!(all(target_os = "linux", not(target_env = "uclibc"))) as u8;

pub fn is_python_process(pid: i32) -> bool {
    #[cfg(all(target_os = "linux", not(target_env = "uclibc")))]
    let result = linux::probe(pid);
    #[cfg(target_os = "macos")]
    let result = macos::probe(pid);
    #[cfg(windows)]
    let result = windows::probe(pid);
    #[cfg(not(any(
        all(target_os = "linux", not(target_env = "uclibc")),
        target_os = "macos",
        windows
    )))]
    let result: Result<bool, std::io::Error> = {
        let _ = pid;
        Err(std::io::ErrorKind::Unsupported.into())
    };
    result.unwrap_or(false)
}
