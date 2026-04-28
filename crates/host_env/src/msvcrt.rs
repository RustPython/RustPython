use alloc::{string::String, vec::Vec};
use std::io;

use crate::crt_fd;
use windows_sys::Win32::System::Diagnostics::Debug;

pub type ErrorMode = u32;

pub const LK_UNLCK: i32 = 0;
pub const LK_LOCK: i32 = 1;
pub const LK_NBLCK: i32 = 2;
pub const LK_RLCK: i32 = 3;
pub const LK_NBRLCK: i32 = 4;
pub const SEM_FAILCRITICALERRORS: ErrorMode = Debug::SEM_FAILCRITICALERRORS;
pub const SEM_NOALIGNMENTFAULTEXCEPT: ErrorMode = Debug::SEM_NOALIGNMENTFAULTEXCEPT;
pub const SEM_NOGPFAULTERRORBOX: ErrorMode = Debug::SEM_NOGPFAULTERRORBOX;
pub const SEM_NOOPENFILEERRORBOX: ErrorMode = Debug::SEM_NOOPENFILEERRORBOX;

unsafe extern "C" {
    fn _getch() -> i32;
    fn _getwch() -> u32;
    fn _getche() -> i32;
    fn _getwche() -> u32;
    fn _putch(c: u32) -> i32;
    fn _putwch(c: u16) -> u32;
    fn _ungetch(c: i32) -> i32;
    fn _ungetwch(c: u32) -> u32;
    fn _locking(fd: i32, mode: i32, nbytes: i64) -> i32;
    fn _heapmin() -> i32;
    fn _kbhit() -> i32;
    fn _setmode(fd: crt_fd::Borrowed<'_>, flags: i32) -> i32;
}

pub fn setmode_binary(fd: crt_fd::Borrowed<'_>) {
    unsafe { suppress_iph!(_setmode(fd, libc::O_BINARY)) };
}

pub fn getch() -> Vec<u8> {
    vec![unsafe { _getch() } as u8]
}

pub fn getwch() -> String {
    let value = unsafe { _getwch() };
    char::from_u32(value).unwrap().to_string()
}

pub fn getche() -> Vec<u8> {
    vec![unsafe { _getche() } as u8]
}

pub fn getwche() -> String {
    let value = unsafe { _getwche() };
    char::from_u32(value).unwrap().to_string()
}

pub fn putch(c: u8) {
    unsafe { suppress_iph!(_putch(c.into())) };
}

pub fn putwch(c: char) {
    unsafe { suppress_iph!(_putwch(c as u16)) };
}

pub fn ungetch(c: u8) -> io::Result<()> {
    let ret = unsafe { suppress_iph!(_ungetch(c as i32)) };
    if ret == -1 {
        Err(io::Error::from_raw_os_error(libc::ENOSPC))
    } else {
        Ok(())
    }
}

pub fn ungetwch(c: char) -> io::Result<()> {
    let ret = unsafe { suppress_iph!(_ungetwch(c as u32)) };
    if ret == 0xFFFF {
        Err(io::Error::from_raw_os_error(libc::ENOSPC))
    } else {
        Ok(())
    }
}

pub fn kbhit() -> i32 {
    unsafe { _kbhit() }
}

pub fn locking(fd: i32, mode: i32, nbytes: i64) -> io::Result<()> {
    let ret = unsafe { suppress_iph!(_locking(fd, mode, nbytes)) };
    if ret == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub fn heapmin() -> io::Result<()> {
    let ret = unsafe { suppress_iph!(_heapmin()) };
    if ret == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub fn setmode(fd: crt_fd::Borrowed<'_>, flags: i32) -> io::Result<i32> {
    let ret = unsafe { suppress_iph!(_setmode(fd, flags)) };
    if ret == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(ret)
    }
}

pub fn open_osfhandle(handle: isize, flags: i32) -> io::Result<i32> {
    let ret = unsafe { suppress_iph!(libc::open_osfhandle(handle, flags)) };
    if ret == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(ret)
    }
}

pub fn get_error_mode() -> u32 {
    unsafe { suppress_iph!(Debug::GetErrorMode()) }
}

pub fn set_error_mode(mode: ErrorMode) -> u32 {
    unsafe { suppress_iph!(Debug::SetErrorMode(mode)) }
}
