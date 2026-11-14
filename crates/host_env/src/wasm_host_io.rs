//! Version 1 of the opt-in, non-WASI embedding descriptor ABI.
//!
//! Descriptors index a per-instance host-owned table, never the host process's
//! raw descriptor namespace. The embedding controls which files are exposed.
//! Results are nonnegative on success; errors are the negated guest errno
//! numbers from errno_wasm, not native host errno values. Read/write return a
//! byte count, seek an absolute position, info a bitmask, and other calls zero.
//! Info bits: 1 directory, 2 native reads-without-waiting classification,
//! 4 terminal. Unknown bits are reserved.
//!
//! Buffers refer to the exported 32-bit guest linear memory. Before allocating
//! or performing I/O the host must validate the entire pointer/length range,
//! including overflow. In particular, an invalid read destination must not
//! consume file or pipe data. Hosts may perform short I/O but never return a
//! count larger than len. Imports are synchronous and may not retain pointers.
//! Close consumes its table entry exactly once, even on failure (including
//! EINTR); neither the guest nor the host may retry closing that entry.
//!
//! Path operations are intentionally absent. This ABI does not provide an OS
//! module, filesystem discovery, process spawning, or a complete test runner.

use std::io;

#[link(wasm_import_module = "rustpython_host_io_v1")]
unsafe extern "C" {
    pub(crate) fn fd_read(fd: i32, ptr: u32, len: u32) -> i64;
    pub(crate) fn fd_write(fd: i32, ptr: u32, len: u32) -> i64;
    pub(crate) fn fd_close(fd: i32) -> i64;
    pub(crate) fn fd_seek(fd: i32, offset: i64, whence: i32) -> i64;
    pub(crate) fn fd_truncate(fd: i32, len: i64) -> i64;
    pub(crate) fn fd_sync(fd: i32) -> i64;
    pub(crate) fn fd_info(fd: i32) -> i64;
}

pub(crate) fn result(value: i64) -> io::Result<i64> {
    if value >= 0 {
        Ok(value)
    } else {
        let errno = value
            .checked_neg()
            .and_then(|n| i32::try_from(n).ok())
            .ok_or_else(invalid_result)?;
        Err(io::Error::from_raw_os_error(errno))
    }
}

pub(crate) fn checked_count(count: i64, len: usize) -> io::Result<usize> {
    usize::try_from(count)
        .ok()
        .filter(|&n| n <= len)
        .ok_or_else(invalid_result)
}

fn invalid_result() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "invalid host descriptor ABI result",
    )
}
