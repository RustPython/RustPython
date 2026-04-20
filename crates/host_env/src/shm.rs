use core::ffi::CStr;
use std::io;

pub fn shm_open(name: &CStr, flags: libc::c_int, mode: libc::c_uint) -> io::Result<libc::c_int> {
    #[cfg(target_os = "freebsd")]
    let mode = mode.try_into().unwrap();

    let fd = unsafe { libc::shm_open(name.as_ptr(), flags, mode) };
    if fd == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(fd)
    }
}

pub fn shm_unlink(name: &CStr) -> io::Result<()> {
    let ret = unsafe { libc::shm_unlink(name.as_ptr()) };
    if ret == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
