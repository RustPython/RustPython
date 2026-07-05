//! POSIX-compatible API for Windows.
//!
//! Python wraps POSIX syscalls such as `mkdir` and `open`. Windows doesn't directly implement
//! these syscalls, but they can be emulated with a mix of the Windows API and the Rust standard
//! library, the latter of which calls the former.

use std::{fs, io, path::Path};

use crate::crt_fd;

#[expect(non_camel_case_types)]
pub type mode_t = u32;

pub fn make_dir(
    dir_fd: Option<crt_fd::Borrowed<'_>>,
    path: &impl AsRef<Path>,
    _mode: mode_t,
) -> io::Result<()> {
    debug_assert!(dir_fd.is_none());
    // TODO: On Windows, Python has an override if the mode is 0o700
    fs::create_dir(path)
}
