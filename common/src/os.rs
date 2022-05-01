// TODO: we can move more os-specific bindings/interfaces from stdlib::{os, posix, nt} to here

use std::io;

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
