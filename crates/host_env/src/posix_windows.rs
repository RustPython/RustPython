//! POSIX-compatible API for Windows.
//!
//! Python wraps POSIX syscalls such as `mkdir` and `open`. Windows doesn't directly implement
//! these syscalls, but they can be emulated with a mix of the Windows API and the Rust standard
//! library, the latter of which calls the former.

use core::hint::cold_path;
use std::{fs, io, path::Path};

use widestring::WideCString;
use windows_sys::Win32::{
    Foundation::FALSE,
    Storage::FileSystem::{MOVE_FILE_FLAGS, MOVEFILE_REPLACE_EXISTING, MoveFileExW},
};

use crate::crt_fd;

pub type RawMode = u32;

pub fn make_dir(
    dir_fd: Option<crt_fd::Borrowed<'_>>,
    path: impl AsRef<Path>,
    _mode: RawMode,
) -> io::Result<()> {
    debug_assert!(dir_fd.is_none());
    // TODO: On Windows, Python has an override if the mode is 0o700
    fs::create_dir(path)
}

/// https://pubs.opengroup.org/onlinepubs/9799919799/functions/rename.html
#[inline]
pub fn rename(
    from: impl AsRef<Path>,
    #[cfg_attr(not(debug_assertions), expect(unused_variables))] from_fd: Option<
        crt_fd::Borrowed<'_>,
    >,
    to: impl AsRef<Path>,
    #[cfg_attr(not(debug_assertions), expect(unused_variables))] to_fd: Option<
        crt_fd::Borrowed<'_>,
    >,
) -> io::Result<()> {
    debug_assert!(from_fd.is_none());
    debug_assert!(to_fd.is_none());

    rename_impl(from, to, 0)
}

/// https://docs.python.org/3/library/os.html#os.replace
///
/// Atomically replace `to` with `from`.
#[inline]
pub fn replace(
    from: impl AsRef<Path>,
    #[cfg_attr(not(debug_assertions), expect(unused_variables))] from_fd: Option<
        crt_fd::Borrowed<'_>,
    >,
    to: impl AsRef<Path>,
    #[cfg_attr(not(debug_assertions), expect(unused_variables))] to_fd: Option<
        crt_fd::Borrowed<'_>,
    >,
) -> io::Result<()> {
    debug_assert!(from_fd.is_none());
    debug_assert!(to_fd.is_none());

    rename_impl(from, to, MOVEFILE_REPLACE_EXISTING)
}

fn rename_impl(
    from: impl AsRef<Path>,
    to: impl AsRef<Path>,
    flags: MOVE_FILE_FLAGS,
) -> io::Result<()> {
    let from = WideCString::from_os_str(from.as_ref())
        .map_err(io::Error::other)?
        .into_vec_with_nul();
    let to = WideCString::from_os_str(to.as_ref())
        .map_err(io::Error::other)?
        .into_vec_with_nul();

    // SAFETY:
    // * from and to are NUL terminated wide strings
    let success = unsafe {
        // Rust's [`std::fs::rename`] is more complicated than CPython's. Rust attempts to use modern APIs
        // where available, such as `FileRenameInfoEx`, which better map to POSIX. CPython simply
        // calls MoveFileExW so we'll do that for parity. However, it may be better to use the new
        // APIs and fall back if possible, especially if they're faster.
        //
        // Unlike POSIX's rename, MoveFileExW does not automatically move between volumes.
        // This is expected behavior in CPython.
        MoveFileExW(from.as_ptr(), to.as_ptr(), flags)
    };

    if success != FALSE {
        Ok(())
    } else {
        cold_path();
        Err(io::Error::last_os_error())
    }
}
