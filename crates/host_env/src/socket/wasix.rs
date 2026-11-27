//! Opt-in WASIX 32-bit socket ABI. This is not part of WASI preview 1.
//!
//! Keep the wire types independent of libc: Python exposes BSD constants while
//! WASIX uses its own address-family, option, flag and errno namespaces.
use super::*;
use core::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    time::Duration,
};
use std::io;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Address {
    family: u8,
    padding: u8,
    bytes: [u8; 18],
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct IpAddress {
    family: u8,
    padding: u8,
    bytes: [u8; 16],
}
#[repr(C)]
struct Iovec {
    base: *const u8,
    len: usize,
}
#[repr(C)]
#[derive(Default)]
struct Fdstat {
    kind: u8,
    padding: u8,
    flags: u16,
    padding2: u32,
    rights: u64,
    inheriting: u64,
}
#[repr(C, align(8))]
#[derive(Clone, Copy, Default)]
struct Subscription {
    userdata: u64,
    tag: u8,
    padding: [u8; 7],
    data: [u8; 32],
}
#[repr(C, align(8))]
#[derive(Clone, Copy, Default)]
struct Event {
    userdata: u64,
    error: u16,
    tag: u8,
    padding: [u8; 5],
    data: [u8; 16],
}

#[link(wasm_import_module = "wasix_32v1")]
unsafe extern "C" {
    fn sock_open(family: u32, kind: u32, protocol: u32, fd: *mut u32) -> u32;
    fn sock_status(fd: u32, status: *mut u8) -> u32;
    fn sock_bind(fd: u32, addr: *const Address) -> u32;
    fn sock_connect(fd: u32, addr: *const Address) -> u32;
    fn sock_listen(fd: u32, backlog: u32) -> u32;
    fn sock_accept(fd: u32, flags: u32, result: *mut u32, addr: *mut Address) -> u32;
    fn sock_send(fd: u32, data: *const Iovec, len: usize, flags: u32, sent: *mut usize) -> u32;
    fn sock_recv(
        fd: u32,
        data: *const Iovec,
        len: usize,
        flags: u32,
        received: *mut usize,
        out_flags: *mut u16,
    ) -> u32;
    fn sock_send_to(
        fd: u32,
        data: *const Iovec,
        len: usize,
        flags: u32,
        addr: *const Address,
        sent: *mut usize,
    ) -> u32;
    fn sock_recv_from(
        fd: u32,
        data: *const Iovec,
        len: usize,
        flags: u32,
        received: *mut usize,
        out_flags: *mut u16,
        addr: *mut Address,
    ) -> u32;
    fn sock_shutdown(fd: u32, how: u32) -> u32;
    fn sock_addr_local(fd: u32, addr: *mut Address) -> u32;
    fn sock_addr_peer(fd: u32, addr: *mut Address) -> u32;
    fn sock_set_opt_flag(fd: u32, option: u32, value: u32) -> u32;
    fn sock_get_opt_flag(fd: u32, option: u32, value: *mut u8) -> u32;
    fn sock_set_opt_size(fd: u32, option: u32, value: u64) -> u32;
    fn sock_get_opt_size(fd: u32, option: u32, value: *mut u64) -> u32;
    fn fd_dup(fd: u32, result: *mut u32) -> u32;
    #[link_name = "resolve"]
    fn wasix_resolve(
        host: *const u8,
        len: usize,
        port: u32,
        addrs: *mut IpAddress,
        capacity: usize,
        count: *mut usize,
    ) -> u32;
}
#[link(wasm_import_module = "wasi_snapshot_preview1")]
unsafe extern "C" {
    fn fd_close(fd: u32) -> u32;
    fn fd_fdstat_get(fd: u32, stat: *mut Fdstat) -> u32;
    fn fd_fdstat_set_flags(fd: u32, flags: u32) -> u32;
    fn poll_oneoff(
        input: *const Subscription,
        output: *mut Event,
        count: usize,
        returned: *mut usize,
    ) -> u32;
}

fn check(errno: u32) -> io::Result<()> {
    if errno == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(errno as i32))
    }
}
fn unsupported() -> io::Error {
    io::Error::from_raw_os_error(58)
} // WASI ENOTSUP
fn invalid() -> io::Error {
    io::Error::from_raw_os_error(28)
} // WASI EINVAL

impl Address {
    fn encode(addr: &SocketAddr) -> io::Result<Self> {
        // WASIX has no fields for an IPv6 flow label or scope identifier.
        if let SocketAddr::V6(addr) = addr
            && (addr.flowinfo() != 0 || addr.scope_id() != 0)
        {
            return Err(unsupported());
        }
        let mut result = Self::default();
        // WASIX input ports are native endian; output ports are network endian.
        result.bytes[..2].copy_from_slice(&addr.port().to_le_bytes());
        match addr.ip() {
            IpAddr::V4(ip) => {
                result.family = 1;
                result.bytes[2..6].copy_from_slice(&ip.octets());
            }
            IpAddr::V6(ip) => {
                result.family = 2;
                result.bytes[2..].copy_from_slice(&ip.octets());
            }
        }
        Ok(result)
    }
    fn decode(self) -> io::Result<SocketAddr> {
        Ok(SocketAddr::new(
            decode_ip(self.family, &self.bytes[2..])?,
            u16::from_be_bytes([self.bytes[0], self.bytes[1]]),
        ))
    }
}
fn decode_ip(family: u8, bytes: &[u8]) -> io::Result<IpAddr> {
    match family {
        1 => Ok(Ipv4Addr::new(bytes[0], bytes[1], bytes[2], bytes[3]).into()),
        2 => Ok(Ipv6Addr::from(<[u8; 16]>::try_from(bytes).map_err(|_| invalid())?).into()),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid WASIX address family",
        )),
    }
}

/// An owned WASIX socket descriptor. Closing a duplicate does not close peers.
#[derive(Debug)]
pub struct Socket {
    fd: u32,
    kind: Option<i32>,
}
impl Drop for Socket {
    fn drop(&mut self) {
        unsafe {
            fd_close(self.fd);
        }
    }
}
impl Socket {
    pub fn new(family: i32, kind: i32, protocol: i32) -> io::Result<Self> {
        let af = match family {
            AF_INET => 1,
            AF_INET6 => 2,
            _ => return Err(io::Error::from_raw_os_error(5)),
        };
        if !matches!(kind, SOCK_STREAM | SOCK_DGRAM) {
            return Err(unsupported());
        }
        if !matches!(
            (kind, protocol),
            (SOCK_STREAM, 0 | IPPROTO_TCP) | (SOCK_DGRAM, 0 | IPPROTO_UDP)
        ) {
            return Err(io::Error::from_raw_os_error(66));
        }
        let mut fd = 0;
        unsafe {
            check(sock_open(af, kind as u32, protocol as u32, &mut fd))?;
        }
        Ok(Self {
            fd,
            kind: Some(kind),
        })
    }
    /// Takes ownership of a valid socket descriptor on success.
    ///
    /// # Safety
    /// The caller must own `fd` and must not close it after this succeeds.
    pub unsafe fn from_raw_fd(fd: i32) -> io::Result<Self> {
        if fd < 0 {
            return Err(io::Error::from_raw_os_error(8));
        }
        let mut status = 0;
        unsafe {
            check(sock_status(fd as u32, &mut status))?;
        }
        Ok(Self {
            fd: fd as u32,
            kind: None,
        })
    }
    pub fn as_raw_fd(&self) -> i32 {
        self.fd as i32
    }
    pub fn into_raw_fd(self) -> i32 {
        let fd = self.fd;
        core::mem::forget(self);
        fd as i32
    }
    /// Closes the descriptor exactly once, including when the host reports an error.
    pub fn close(self) -> io::Result<()> {
        let fd = self.into_raw_fd();
        unsafe { check(fd_close(fd as u32)) }
    }
    pub fn try_clone(&self) -> io::Result<Self> {
        let mut fd = 0;
        unsafe {
            check(fd_dup(self.fd, &mut fd))?;
        }
        Ok(Self {
            fd,
            kind: self.kind,
        })
    }
    pub fn bind(&self, addr: &SocketAddr) -> io::Result<()> {
        let addr = Address::encode(addr)?;
        unsafe { check(sock_bind(self.fd, &addr)) }
    }
    pub fn connect(&self, addr: &SocketAddr) -> io::Result<()> {
        let addr = Address::encode(addr)?;
        unsafe { check(sock_connect(self.fd, &addr)) }
    }
    pub fn listen(&self, backlog: i32) -> io::Result<()> {
        unsafe { check(sock_listen(self.fd, backlog.max(0) as u32)) }
    }
    pub fn accept(&self) -> io::Result<(Self, SocketAddr)> {
        let mut stat = Fdstat::default();
        unsafe {
            check(fd_fdstat_get(self.fd, &mut stat))?;
        }
        let mut fd = 0;
        let mut addr = Address::default();
        unsafe {
            check(sock_accept(
                self.fd,
                u32::from(stat.flags & 4),
                &mut fd,
                &mut addr,
            ))?;
        }
        let socket = Self {
            fd,
            kind: Some(SOCK_STREAM),
        };
        Ok((socket, addr.decode()?))
    }
    pub fn send(&self, data: &[u8], flags: i32) -> io::Result<usize> {
        if flags != 0 {
            return Err(unsupported());
        }
        let iov = Iovec {
            base: data.as_ptr(),
            len: data.len(),
        };
        let mut size = 0;
        unsafe {
            check(sock_send(self.fd, &iov, 1, 0, &mut size))?;
        }
        Ok(size)
    }
    pub fn recv(&self, data: &mut [u8], flags: i32) -> io::Result<usize> {
        let flags = recv_flags(flags)?;
        let iov = Iovec {
            base: data.as_mut_ptr(),
            len: data.len(),
        };
        let mut size = 0;
        let mut out_flags = 0;
        unsafe {
            check(sock_recv(
                self.fd,
                &iov,
                1,
                flags,
                &mut size,
                &mut out_flags,
            ))?;
        }
        Ok(size)
    }
    pub fn send_to(&self, data: &[u8], flags: i32, addr: &SocketAddr) -> io::Result<usize> {
        if flags != 0 {
            return Err(unsupported());
        }
        let addr = Address::encode(addr)?;
        let iov = Iovec {
            base: data.as_ptr(),
            len: data.len(),
        };
        let mut size = 0;
        unsafe {
            check(sock_send_to(self.fd, &iov, 1, 0, &addr, &mut size))?;
        }
        Ok(size)
    }
    pub fn recv_from(&self, data: &mut [u8], flags: i32) -> io::Result<(usize, SocketAddr)> {
        let flags = recv_flags(flags)?;
        let iov = Iovec {
            base: data.as_mut_ptr(),
            len: data.len(),
        };
        let mut size = 0;
        let mut out_flags = 0;
        let mut addr = Address::default();
        unsafe {
            check(sock_recv_from(
                self.fd,
                &iov,
                1,
                flags,
                &mut size,
                &mut out_flags,
                &mut addr,
            ))?;
        }
        Ok((size, addr.decode()?))
    }
    pub fn shutdown(&self, how: i32) -> io::Result<()> {
        let flags = match how {
            SHUT_RD => 1,
            SHUT_WR => 2,
            SHUT_RDWR => 3,
            _ => return Err(invalid()),
        };
        unsafe { check(sock_shutdown(self.fd, flags)) }
    }
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        let mut addr = Address::default();
        unsafe {
            check(sock_addr_local(self.fd, &mut addr))?;
        }
        addr.decode()
    }
    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        let mut addr = Address::default();
        unsafe {
            check(sock_addr_peer(self.fd, &mut addr))?;
        }
        addr.decode()
    }
    pub fn set_nonblocking(&self, enabled: bool) -> io::Result<()> {
        let mut stat = Fdstat::default();
        unsafe {
            check(fd_fdstat_get(self.fd, &mut stat))?;
            check(fd_fdstat_set_flags(
                self.fd,
                u32::from(if enabled {
                    stat.flags | 4
                } else {
                    stat.flags & !4
                }),
            ))
        }
    }
    pub fn set_option(&self, level: i32, name: i32, value: i32) -> io::Result<()> {
        let (option, size) = option(level, name)?;
        if (level == SOL_SOCKET && name == SO_ERROR) || value < 0 && size {
            return Err(invalid());
        }
        unsafe {
            check(if size {
                sock_set_opt_size(self.fd, option, value as u64)
            } else {
                sock_set_opt_flag(self.fd, option, u32::from(value != 0))
            })
        }
    }
    pub fn get_option(&self, level: i32, name: i32) -> io::Result<i32> {
        if level == SOL_SOCKET && name == SO_TYPE {
            if let Some(kind) = self.kind {
                return Ok(kind);
            }
            let mut stat = Fdstat::default();
            unsafe {
                check(fd_fdstat_get(self.fd, &mut stat))?;
            }
            return match stat.kind {
                5 => Ok(SOCK_DGRAM),
                6 => Ok(SOCK_STREAM),
                _ => Err(unsupported()),
            };
        }
        let (option, size) = option(level, name)?;
        if size {
            let mut value = 0;
            unsafe {
                check(sock_get_opt_size(self.fd, option, &mut value))?;
            }
            i32::try_from(value).map_err(|_| io::Error::from_raw_os_error(61))
        } else {
            let mut value = 0;
            unsafe {
                check(sock_get_opt_flag(self.fd, option, &mut value))?;
            }
            Ok(i32::from(value))
        }
    }
    /// Waits for readiness, returning false only when the clock expires.
    pub fn wait(&self, read: bool, write: bool, timeout: Duration) -> io::Result<bool> {
        let mut subscriptions = [Subscription::default(); 3];
        let mut count = 0;
        for (enabled, tag) in [(read, 1), (write, 2)] {
            if enabled {
                subscriptions[count].userdata = 1;
                subscriptions[count].tag = tag;
                subscriptions[count].data[..4].copy_from_slice(&self.fd.to_le_bytes());
                count += 1;
            }
        }
        if count == 0 {
            return Err(invalid());
        }
        // A relative monotonic clock subscription. Payload offsets follow WASI
        // subscription_clock_t (u32 id, padding, u64 timeout/precision, u16 flags).
        subscriptions[count].data[..4].copy_from_slice(&1u32.to_le_bytes());
        let nanos = timeout.as_nanos().min(u64::MAX as u128) as u64;
        subscriptions[count].data[8..16].copy_from_slice(&nanos.to_le_bytes());
        subscriptions[count].data[16..24].copy_from_slice(&1u64.to_le_bytes());
        count += 1;
        let mut events = [Event::default(); 3];
        let mut returned = 0;
        unsafe {
            check(poll_oneoff(
                subscriptions.as_ptr(),
                events.as_mut_ptr(),
                count,
                &mut returned,
            ))?;
        }
        if returned > events.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid WASI poll event count",
            ));
        }
        let mut ready = false;
        for event in &events[..returned] {
            check(u32::from(event.error))?;
            ready |= event.userdata == 1;
        }
        Ok(ready)
    }
}
fn recv_flags(flags: i32) -> io::Result<u32> {
    if flags & !MSG_PEEK != 0 {
        return Err(unsupported());
    }
    Ok(u32::from(flags & MSG_PEEK != 0))
}
fn option(level: i32, name: i32) -> io::Result<(u32, bool)> {
    match (level, name) {
        (SOL_SOCKET, SO_REUSEADDR) => Ok((2, false)),
        (SOL_SOCKET, SO_BROADCAST) => Ok((6, false)),
        (SOL_SOCKET, SO_KEEPALIVE) => Ok((12, false)),
        (SOL_SOCKET, SO_RCVBUF) => Ok((15, true)),
        (SOL_SOCKET, SO_SNDBUF) => Ok((16, true)),
        (SOL_SOCKET, SO_ERROR) => Ok((11, true)),
        (IPPROTO_TCP, TCP_NODELAY) => Ok((3, false)),
        _ => Err(io::Error::from_raw_os_error(50)), // WASI ENOPROTOOPT
    }
}

pub fn resolve(host: &str, port: u16) -> io::Result<Vec<SocketAddr>> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(vec![SocketAddr::new(ip, port)]);
    }
    if host.as_bytes().contains(&0) {
        return Err(invalid());
    }
    let mut capacity = 16;
    loop {
        let mut addresses = vec![IpAddress::default(); capacity];
        let mut count = 0;
        unsafe {
            check(wasix_resolve(
                host.as_ptr(),
                host.len(),
                u32::from(port),
                addresses.as_mut_ptr(),
                addresses.len(),
                &mut count,
            ))?;
        }
        if count > capacity {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid WASIX DNS result count",
            ));
        }
        if count == capacity && capacity < 4096 {
            capacity *= 2;
            continue;
        }
        return addresses[..count]
            .iter()
            .map(|addr| Ok(SocketAddr::new(decode_ip(addr.family, &addr.bytes)?, port)))
            .collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_layout() {
        assert_eq!(size_of::<Address>(), 20);
        assert_eq!(size_of::<IpAddress>(), 18);
        assert_eq!(size_of::<Fdstat>(), 24);
        assert_eq!(size_of::<Subscription>(), 48);
        assert_eq!(size_of::<Event>(), 32);
        assert_eq!(core::mem::offset_of!(Subscription, data), 16);
        assert_eq!(core::mem::offset_of!(Event, error), 8);
    }

    #[test]
    fn address_byte_order() {
        for text in ["127.0.0.1:12345", "[2001:db8::1]:443"] {
            let address: SocketAddr = text.parse().unwrap();
            let mut wire = Address::encode(&address).unwrap();
            assert_eq!(wire.bytes[..2], address.port().to_le_bytes());
            // WASIX returns ports in network order, unlike its input format.
            wire.bytes[..2].copy_from_slice(&address.port().to_be_bytes());
            assert_eq!(wire.decode().unwrap(), address);
        }
    }

    #[test]
    fn unsupported_fields_and_flags() {
        let address = SocketAddr::V6(core::net::SocketAddrV6::new(Ipv6Addr::LOCALHOST, 80, 0, 1));
        assert_eq!(
            Address::encode(&address).err().unwrap().raw_os_error(),
            Some(58)
        );
        assert_eq!(recv_flags(MSG_PEEK).unwrap(), 1);
        assert!(recv_flags(MSG_OOB).is_err());
        assert!(option(SOL_SOCKET, 999).is_err());
    }
}
