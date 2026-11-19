//! WASI p2 uses wasi-libc's socket ABI, never Linux placeholder constants.
pub use libc::*;
pub use socket2 as raw;
use std::io;
#[path = "wasi_dns.rs"]
pub mod dns;
pub use dns::gai_error_string;

// wasi-libc netdb.h constants absent from the Rust libc declarations.
pub const AI_PASSIVE: i32 = 0x01;
pub const AI_CANONNAME: i32 = 0x02;
pub const AI_NUMERICHOST: i32 = 0x04;
pub const AI_V4MAPPED: i32 = 0x08;
pub const AI_ALL: i32 = 0x10;
pub const AI_ADDRCONFIG: i32 = 0x20;
pub const AI_NUMERICSERV: i32 = 0x400;
pub const NI_NUMERICHOST: i32 = 1;
pub const NI_NUMERICSERV: i32 = 2;
pub const NI_NOFQDN: i32 = 4;
pub const NI_NAMEREQD: i32 = 8;
pub const NI_DGRAM: i32 = 16;
pub const NI_MAXHOST: usize = 255;
pub const EAI_NONAME: i32 = -2;
pub const INADDR_ANY: u32 = 0;
pub const INADDR_BROADCAST: u32 = u32::MAX;
pub const INADDR_LOOPBACK: u32 = 0x7f00_0001;
pub const INADDR_NONE: u32 = u32::MAX;

fn checked(result: i32) -> io::Result<()> {
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
pub fn close_socket_ignore_connreset(fd: i32) -> io::Result<()> {
    let result = checked(unsafe { libc::close(fd) });
    match result {
        Err(error) if error.raw_os_error() == Some(libc::ECONNRESET) => Ok(()),
        result => result,
    }
}
pub fn getsockopt_int(fd: i32, level: i32, name: i32) -> io::Result<i32> {
    let mut value = 0_i32;
    let mut len = size_of::<i32>() as libc::socklen_t;
    checked(unsafe {
        libc::getsockopt(fd, level, name, (&mut value as *mut i32).cast(), &mut len)
    })?;
    if len as usize != size_of::<i32>() {
        return Err(io::Error::other("invalid integer socket option length"));
    }
    Ok(value)
}
pub fn getsockopt_bytes(fd: i32, level: i32, name: i32, len: usize) -> io::Result<Vec<u8>> {
    let mut bytes = vec![0; len];
    let mut output = libc::socklen_t::try_from(len).map_err(|_| io::ErrorKind::InvalidInput)?;
    checked(unsafe { libc::getsockopt(fd, level, name, bytes.as_mut_ptr().cast(), &mut output) })?;
    if output as usize > len {
        return Err(io::Error::other("invalid socket option length"));
    }
    bytes.truncate(output as usize);
    Ok(bytes)
}
pub fn setsockopt_bytes(fd: i32, level: i32, name: i32, bytes: &[u8]) -> io::Result<()> {
    let len = libc::socklen_t::try_from(bytes.len()).map_err(|_| io::ErrorKind::InvalidInput)?;
    checked(unsafe { libc::setsockopt(fd, level, name, bytes.as_ptr().cast(), len) })
}
pub fn setsockopt_int(fd: i32, level: i32, name: i32, value: i32) -> io::Result<()> {
    setsockopt_bytes(fd, level, name, &value.to_ne_bytes())
}
pub fn setsockopt_none(fd: i32, level: i32, name: i32, len: u32) -> io::Result<()> {
    checked(unsafe { libc::setsockopt(fd, level, name, std::ptr::null(), len) })
}

// wasi-libc sys/socket.h and netdb.h (not provided by libc's Rust bindings).
pub const MSG_WAITALL: i32 = 0x0100;
pub const MSG_TRUNC: i32 = 0x0020;
pub const SOMAXCONN: i32 = 128;
pub const EAI_BADFLAGS: i32 = -1;
pub const EAI_AGAIN: i32 = -3;
pub const EAI_FAIL: i32 = -4;
pub const EAI_FAMILY: i32 = -6;
pub const EAI_SOCKTYPE: i32 = -7;
pub const EAI_SERVICE: i32 = -8;
pub const EAI_MEMORY: i32 = -10;
pub const EAI_OVERFLOW: i32 = -12;
