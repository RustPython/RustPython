//! Socket operations require an explicitly enabled networking host ABI.
use core::{net::SocketAddr, time::Duration};
use std::io;

// No constructor succeeds, and the private field prevents external construction.
#[derive(Debug)]
pub struct Socket {
    _private: (),
}
fn unsupported<T>() -> io::Result<T> {
    Err(io::Error::from_raw_os_error(crate::errno::errors::ENOTSUP))
}
impl Socket {
    pub fn new(_family: i32, _kind: i32, _protocol: i32) -> io::Result<Self> {
        unsupported()
    }
    /// # Safety
    /// The caller must own the descriptor. This backend never takes ownership.
    pub unsafe fn from_raw_fd(_fd: i32) -> io::Result<Self> {
        unsupported()
    }
    pub fn as_raw_fd(&self) -> i32 {
        unreachable!("unsupported sockets cannot be constructed")
    }
    pub fn into_raw_fd(self) -> i32 {
        unreachable!("unsupported sockets cannot be constructed")
    }
    pub fn close(self) -> io::Result<()> {
        unreachable!("unsupported sockets cannot be constructed")
    }
    pub fn try_clone(&self) -> io::Result<Self> {
        unsupported()
    }
    pub fn bind(&self, _addr: &SocketAddr) -> io::Result<()> {
        unsupported()
    }
    pub fn connect(&self, _addr: &SocketAddr) -> io::Result<()> {
        unsupported()
    }
    pub fn listen(&self, _backlog: i32) -> io::Result<()> {
        unsupported()
    }
    pub fn accept(&self) -> io::Result<(Self, SocketAddr)> {
        unsupported()
    }
    pub fn send(&self, _data: &[u8], _flags: i32) -> io::Result<usize> {
        unsupported()
    }
    pub fn recv(&self, _data: &mut [u8], _flags: i32) -> io::Result<usize> {
        unsupported()
    }
    pub fn send_to(&self, _data: &[u8], _flags: i32, _addr: &SocketAddr) -> io::Result<usize> {
        unsupported()
    }
    pub fn recv_from(&self, _data: &mut [u8], _flags: i32) -> io::Result<(usize, SocketAddr)> {
        unsupported()
    }
    pub fn shutdown(&self, _how: i32) -> io::Result<()> {
        unsupported()
    }
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        unsupported()
    }
    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        unsupported()
    }
    pub fn set_nonblocking(&self, _enabled: bool) -> io::Result<()> {
        unsupported()
    }
    pub fn get_option(&self, _level: i32, _name: i32) -> io::Result<i32> {
        unsupported()
    }
    pub fn set_option(&self, _level: i32, _name: i32, _value: i32) -> io::Result<()> {
        unsupported()
    }
    pub fn wait(&self, _read: bool, _write: bool, _timeout: Duration) -> io::Result<bool> {
        unsupported()
    }
}
pub fn resolve(_host: &str, _port: u16) -> io::Result<Vec<SocketAddr>> {
    unsupported()
}
