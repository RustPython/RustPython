use core::ffi::CStr;
use std::io;

use crate::os::CheckLibcResult;

pub fn shm_open(name: &CStr, flags: libc::c_int, mode: libc::c_uint) -> io::Result<libc::c_int> {
    #[cfg(target_os = "freebsd")]
    let mode = mode.try_into().unwrap();

    unsafe { libc::shm_open(name.as_ptr(), flags, mode) }.check_libc_neg()
}

pub fn shm_unlink(name: &CStr) -> io::Result<()> {
    unsafe { libc::shm_unlink(name.as_ptr()) }.check_libc_neg()?;
    Ok(())
}
