use core::ffi::CStr;
use std::io;

use rustix::{
    fd::{IntoRawFd, OwnedFd},
    fs, shm,
};

pub use libc::mode_t;

pub fn shm_open(name: &CStr, flags: libc::c_int, mode: libc::mode_t) -> io::Result<libc::c_int> {
    // FreeBSD implements shared memory differently from Linux so NO_FOLLOW isn't set or needed.
    #[cfg(not(target_os = "linux"))]
    let flags = shm::OFlags::from_bits_retain(flags as _);

    // Glibc and musl both set NOFOLLOW.
    #[cfg(target_os = "linux")]
    let flags = shm::OFlags::from_bits_retain(flags as u32 | fs::OFlags::NOFOLLOW.bits());

    let mode = fs::Mode::from_bits_retain(mode);

    shm::open(name, flags, mode)
        .map(OwnedFd::into_raw_fd)
        .map_err(Into::into)
}

pub fn shm_unlink(name: &CStr) -> io::Result<()> {
    shm::unlink(name).map_err(Into::into)
}
