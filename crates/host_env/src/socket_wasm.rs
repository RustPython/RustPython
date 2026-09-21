//! Socket constant surface for `wasm32-unknown-unknown`.
//!
//! There are no BSD sockets on this target. The numbers are the Linux ABI
//! `Lib/socket.py` expects so the stdlib can import.

pub const AF_UNSPEC: i32 = 0;
pub const AF_UNIX: i32 = 1;
pub const AF_INET: i32 = 2;
pub const AF_INET6: i32 = 10;

pub const SOCK_STREAM: i32 = 1;
pub const SOCK_DGRAM: i32 = 2;
pub const SOCK_RAW: i32 = 3;

pub const SOL_SOCKET: i32 = 1;
pub const SO_REUSEADDR: i32 = 2;
pub const SO_TYPE: i32 = 3;
pub const SO_ERROR: i32 = 4;
pub const SO_BROADCAST: i32 = 6;
pub const SO_KEEPALIVE: i32 = 9;
pub const SO_RCVBUF: i32 = 8;
pub const SO_SNDBUF: i32 = 7;

pub const IPPROTO_IP: i32 = 0;
pub const IPPROTO_TCP: i32 = 6;
pub const IPPROTO_UDP: i32 = 17;
pub const IPPROTO_IPV6: i32 = 41;
pub const SOL_TCP: i32 = IPPROTO_TCP;

pub const SHUT_RD: i32 = 0;
pub const SHUT_WR: i32 = 1;
pub const SHUT_RDWR: i32 = 2;

pub const MSG_OOB: i32 = 1;
pub const MSG_PEEK: i32 = 2;
pub const MSG_DONTROUTE: i32 = 4;

pub const AI_PASSIVE: i32 = 1;
pub const AI_CANONNAME: i32 = 2;
pub const AI_NUMERICHOST: i32 = 4;
pub const AI_NUMERICSERV: i32 = 8;
pub const AI_ADDRCONFIG: i32 = 32;

pub const NI_NUMERICHOST: i32 = 1;
pub const NI_NUMERICSERV: i32 = 2;
pub const NI_NOFQDN: i32 = 4;
pub const NI_NAMEREQD: i32 = 8;
pub const NI_DGRAM: i32 = 16;

pub const INADDR_ANY: u32 = 0;
pub const INADDR_LOOPBACK: u32 = 0x7f00_0001;
pub const INADDR_BROADCAST: u32 = 0xffff_ffff;
pub const INADDR_NONE: u32 = 0xffff_ffff;
pub const IPPORT_RESERVED: i32 = 1024;
pub const IPPORT_USERRESERVED: i32 = 5000;
pub const TCP_NODELAY: i32 = 1;
