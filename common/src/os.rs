// TODO: we can move more os-specific bindings/interfaces from stdlib::{os, posix, nt} to here

use std::{io, str::Utf8Error};

#[cfg(windows)]
pub fn errno() -> io::Error {
    let err = io::Error::last_os_error();
    // FIXME: probably not ideal, we need a bigger dichotomy between GetLastError and errno
    if err.raw_os_error() == Some(0) {
        extern "C" {
            fn _get_errno(pValue: *mut i32) -> i32;
        }
        let mut e = 0;
        unsafe { suppress_iph!(_get_errno(&mut e)) };
        io::Error::from_raw_os_error(e)
    } else {
        err
    }
}
#[cfg(not(windows))]
pub fn errno() -> io::Error {
    io::Error::last_os_error()
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
