use crate::common::cell::{PyRwLock, PyRwLockReadGuard, PyRwLockWriteGuard};
use std::io::{self, prelude::*};
use std::net::{Ipv4Addr, Shutdown, SocketAddr, ToSocketAddrs};
use std::time::Duration;

use byteorder::{BigEndian, ByteOrder};
use crossbeam_utils::atomic::AtomicCell;
use gethostname::gethostname;
#[cfg(all(unix, not(target_os = "redox")))]
use nix::unistd::sethostname;
use socket2::{Domain, Protocol, Socket, Type as SocketType};

use crate::byteslike::PyBytesLike;
use crate::exceptions::{IntoPyException, PyBaseExceptionRef};
use crate::function::{OptionalArg, PyFuncArgs};
use crate::obj::objbytearray::PyByteArrayRef;
use crate::obj::objbytes::PyBytesRef;
use crate::obj::objstr::{PyString, PyStringRef};
use crate::obj::objtuple::PyTupleRef;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{
    BorrowValue, Either, IntoPyObject, PyClassImpl, PyObjectRef, PyRef, PyResult, PyValue,
    TryFromObject,
};
use crate::vm::VirtualMachine;

#[cfg(unix)]
type RawSocket = std::os::unix::io::RawFd;
#[cfg(windows)]
type RawSocket = std::os::windows::raw::SOCKET;

#[cfg(unix)]
mod c {
    pub use libc::*;
    // https://gitlab.redox-os.org/redox-os/relibc/-/blob/master/src/header/netdb/mod.rs
    #[cfg(target_os = "redox")]
    pub const AI_PASSIVE: c_int = 0x01;
    #[cfg(target_os = "redox")]
    pub const AI_ALL: c_int = 0x10;
    // https://gitlab.redox-os.org/redox-os/relibc/-/blob/master/src/header/sys_socket/constants.rs
    #[cfg(target_os = "redox")]
    pub const SO_TYPE: c_int = 3;
    #[cfg(target_os = "redox")]
    pub const MSG_OOB: c_int = 1;
    #[cfg(target_os = "redox")]
    pub const MSG_WAITALL: c_int = 256;
}
#[cfg(windows)]
mod c {
    pub use winapi::shared::ws2def::*;
    pub use winapi::um::winsock2::{
        SD_BOTH as SHUT_RDWR, SD_RECEIVE as SHUT_RD, SD_SEND as SHUT_WR, SOCK_DGRAM, SOCK_RAW,
        SOCK_RDM, SOCK_STREAM, SOL_SOCKET, SO_BROADCAST, SO_REUSEADDR, SO_TYPE, *,
    };
}

#[pyclass]
#[derive(Debug)]
pub struct PySocket {
    kind: AtomicCell<i32>,
    family: AtomicCell<i32>,
    proto: AtomicCell<i32>,
    sock: PyRwLock<Socket>,
}

impl PyValue for PySocket {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("_socket", "socket")
    }
}

pub type PySocketRef = PyRef<PySocket>;

#[pyimpl(flags(BASETYPE))]
impl PySocket {
    fn sock(&self) -> PyRwLockReadGuard<'_, Socket> {
        self.sock.read()
    }

    fn sock_mut(&self) -> PyRwLockWriteGuard<'_, Socket> {
        self.sock.write()
    }

    #[pyslot]
    fn tp_new(cls: PyClassRef, _args: PyFuncArgs, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        PySocket {
            kind: AtomicCell::default(),
            family: AtomicCell::default(),
            proto: AtomicCell::default(),
            sock: PyRwLock::new(invalid_sock()),
        }
        .into_ref_with_type(vm, cls)
    }

    #[pymethod(name = "__init__")]
    fn init(
        &self,
        family: i32,
        socket_kind: i32,
        proto: i32,
        fileno: Option<RawSocket>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let sock = if let Some(fileno) = fileno {
            #[cfg(unix)]
            {
                use std::os::unix::io::FromRawFd;
                unsafe { Socket::from_raw_fd(fileno) }
            }
            #[cfg(windows)]
            {
                use std::os::windows::io::FromRawSocket;
                unsafe { Socket::from_raw_socket(fileno) }
            }
        } else {
            let sock = Socket::new(
                Domain::from(family),
                SocketType::from(socket_kind),
                Some(Protocol::from(proto)),
            )
            .map_err(|err| convert_sock_error(vm, err))?;

            self.family.store(family);
            self.kind.store(socket_kind);
            self.proto.store(proto);
            sock
        };
        *self.sock.write() = sock;
        Ok(())
    }

    #[pymethod]
    fn connect(&self, address: Address, vm: &VirtualMachine) -> PyResult<()> {
        let sock_addr = get_addr(vm, address)?;
        let res = if let Some(duration) = self.sock().read_timeout().unwrap() {
            self.sock().connect_timeout(&sock_addr, duration)
        } else {
            self.sock().connect(&sock_addr)
        };
        res.map_err(|err| convert_sock_error(vm, err))
    }

    #[pymethod]
    fn bind(&self, address: Address, vm: &VirtualMachine) -> PyResult<()> {
        let sock_addr = get_addr(vm, address)?;
        self.sock()
            .bind(&sock_addr)
            .map_err(|err| convert_sock_error(vm, err))
    }

    #[pymethod]
    fn listen(&self, backlog: OptionalArg<i32>, vm: &VirtualMachine) -> PyResult<()> {
        let backlog = backlog.unwrap_or(128);
        let backlog = if backlog < 0 { 0 } else { backlog };
        self.sock()
            .listen(backlog)
            .map_err(|err| convert_sock_error(vm, err))
    }

    #[pymethod]
    fn _accept(&self, vm: &VirtualMachine) -> PyResult<(RawSocket, AddrTuple)> {
        let (sock, addr) = self
            .sock()
            .accept()
            .map_err(|err| convert_sock_error(vm, err))?;

        let fd = into_sock_fileno(sock);
        Ok((fd, get_addr_tuple(addr)))
    }

    #[pymethod]
    fn recv(&self, bufsize: usize, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        let mut buffer = vec![0u8; bufsize];
        let n = self
            .sock()
            .recv(&mut buffer)
            .map_err(|err| convert_sock_error(vm, err))?;
        buffer.truncate(n);
        Ok(buffer)
    }

    #[pymethod]
    fn recv_into(&self, buf: PyByteArrayRef, vm: &VirtualMachine) -> PyResult<usize> {
        let mut buffer = buf.borrow_value_mut();
        self.sock()
            .recv(&mut buffer.elements)
            .map_err(|err| convert_sock_error(vm, err))
    }

    #[pymethod]
    fn recvfrom(&self, bufsize: usize, vm: &VirtualMachine) -> PyResult<(Vec<u8>, AddrTuple)> {
        let mut buffer = vec![0u8; bufsize];
        let (n, addr) = self
            .sock()
            .recv_from(&mut buffer)
            .map_err(|err| convert_sock_error(vm, err))?;
        buffer.truncate(n);
        Ok((buffer, get_addr_tuple(addr)))
    }

    #[pymethod]
    fn send(&self, bytes: PyBytesLike, vm: &VirtualMachine) -> PyResult<usize> {
        bytes
            .with_ref(|b| self.sock().send(b))
            .map_err(|err| convert_sock_error(vm, err))
    }

    #[pymethod]
    fn sendall(&self, bytes: PyBytesLike, vm: &VirtualMachine) -> PyResult<()> {
        bytes
            .with_ref(|b| self.sock_mut().write_all(b))
            .map_err(|err| convert_sock_error(vm, err))
    }

    #[pymethod]
    fn sendto(&self, bytes: PyBytesLike, address: Address, vm: &VirtualMachine) -> PyResult<()> {
        let addr = get_addr(vm, address)?;
        bytes
            .with_ref(|b| self.sock().send_to(b, &addr))
            .map_err(|err| convert_sock_error(vm, err))?;
        Ok(())
    }

    #[pymethod]
    fn close(&self) {
        *self.sock_mut() = invalid_sock();
    }
    #[pymethod]
    fn detach(&self) -> RawSocket {
        into_sock_fileno(std::mem::replace(&mut *self.sock_mut(), invalid_sock()))
    }

    #[pymethod]
    fn fileno(&self) -> RawSocket {
        sock_fileno(&self.sock())
    }

    #[pymethod]
    fn getsockname(&self, vm: &VirtualMachine) -> PyResult<AddrTuple> {
        let addr = self
            .sock()
            .local_addr()
            .map_err(|err| convert_sock_error(vm, err))?;

        Ok(get_addr_tuple(addr))
    }
    #[pymethod]
    fn getpeername(&self, vm: &VirtualMachine) -> PyResult<AddrTuple> {
        let addr = self
            .sock()
            .peer_addr()
            .map_err(|err| convert_sock_error(vm, err))?;

        Ok(get_addr_tuple(addr))
    }

    #[pymethod]
    fn gettimeout(&self, vm: &VirtualMachine) -> PyResult<Option<f64>> {
        let dur = self
            .sock()
            .read_timeout()
            .map_err(|err| convert_sock_error(vm, err))?;
        Ok(dur.map(|d| d.as_secs_f64()))
    }

    #[pymethod]
    fn setblocking(&self, block: bool, vm: &VirtualMachine) -> PyResult<()> {
        self.sock()
            .set_nonblocking(!block)
            .map_err(|err| convert_sock_error(vm, err))
    }

    #[pymethod]
    fn getblocking(&self, vm: &VirtualMachine) -> PyResult<bool> {
        Ok(self.gettimeout(vm)?.map_or(false, |t| t == 0.0))
    }

    #[pymethod]
    fn settimeout(&self, timeout: Option<Duration>, vm: &VirtualMachine) -> PyResult<()> {
        // timeout is None: blocking, no timeout
        // timeout is 0: non-blocking, no timeout
        // otherwise: timeout is timeout, don't change blocking
        let (block, timeout) = match timeout {
            None => (Some(true), None),
            Some(d) if d == Duration::from_secs(0) => (Some(false), None),
            Some(d) => (None, Some(d)),
        };
        self.sock()
            .set_read_timeout(timeout)
            .map_err(|err| convert_sock_error(vm, err))?;
        self.sock()
            .set_write_timeout(timeout)
            .map_err(|err| convert_sock_error(vm, err))?;
        if let Some(blocking) = block {
            self.sock()
                .set_nonblocking(!blocking)
                .map_err(|err| convert_sock_error(vm, err))?;
        }
        Ok(())
    }

    #[pymethod]
    fn getsockopt(
        &self,
        level: i32,
        name: i32,
        buflen: OptionalArg<i32>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let fd = sock_fileno(&self.sock()) as _;
        let buflen = buflen.unwrap_or(0);
        if buflen == 0 {
            let mut flag: libc::c_int = 0;
            let mut flagsize = std::mem::size_of::<libc::c_int>() as _;
            let ret = unsafe {
                c::getsockopt(
                    fd,
                    level,
                    name,
                    &mut flag as *mut libc::c_int as *mut _,
                    &mut flagsize,
                )
            };
            if ret < 0 {
                Err(convert_sock_error(vm, io::Error::last_os_error()))
            } else {
                Ok(vm.ctx.new_int(flag))
            }
        } else {
            if buflen <= 0 || buflen > 1024 {
                return Err(vm.new_os_error("getsockopt buflen out of range".to_owned()));
            }
            let mut buf = vec![0u8; buflen as usize];
            let mut buflen = buflen as _;
            let ret =
                unsafe { c::getsockopt(fd, level, name, buf.as_mut_ptr() as *mut _, &mut buflen) };
            buf.truncate(buflen as usize);
            if ret < 0 {
                Err(convert_sock_error(vm, io::Error::last_os_error()))
            } else {
                Ok(vm.ctx.new_bytes(buf))
            }
        }
    }

    #[pymethod]
    fn setsockopt(
        &self,
        level: i32,
        name: i32,
        value: Option<Either<PyBytesLike, i32>>,
        optlen: OptionalArg<u32>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let fd = sock_fileno(&self.sock()) as _;
        let ret = match (value, optlen) {
            (Some(Either::A(b)), OptionalArg::Missing) => b.with_ref(|b| unsafe {
                c::setsockopt(fd, level, name, b.as_ptr() as *const _, b.len() as _)
            }),
            (Some(Either::B(ref val)), OptionalArg::Missing) => unsafe {
                c::setsockopt(
                    fd,
                    level,
                    name,
                    val as *const i32 as *const _,
                    std::mem::size_of::<i32>() as _,
                )
            },
            (None, OptionalArg::Present(optlen)) => unsafe {
                c::setsockopt(fd, level, name, std::ptr::null(), optlen as _)
            },
            _ => {
                return Err(
                    vm.new_type_error("expected the value arg xor the optlen arg".to_owned())
                );
            }
        };
        if ret < 0 {
            Err(convert_sock_error(vm, io::Error::last_os_error()))
        } else {
            Ok(())
        }
    }

    #[pymethod]
    fn shutdown(&self, how: i32, vm: &VirtualMachine) -> PyResult<()> {
        let how = match how {
            c::SHUT_RD => Shutdown::Read,
            c::SHUT_WR => Shutdown::Write,
            c::SHUT_RDWR => Shutdown::Both,
            _ => {
                return Err(
                    vm.new_value_error("`how` must be SHUT_RD, SHUT_WR, or SHUT_RDWR".to_owned())
                )
            }
        };
        self.sock()
            .shutdown(how)
            .map_err(|err| convert_sock_error(vm, err))
    }

    #[pyproperty(name = "type")]
    fn kind(&self) -> i32 {
        self.kind.load()
    }
    #[pyproperty]
    fn family(&self) -> i32 {
        self.family.load()
    }
    #[pyproperty]
    fn proto(&self) -> i32 {
        self.proto.load()
    }
}

impl io::Read for PySocketRef {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        <Socket as io::Read>::read(&mut self.sock_mut(), buf)
    }
}
impl io::Write for PySocketRef {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        <Socket as io::Write>::write(&mut self.sock_mut(), buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        <Socket as io::Write>::flush(&mut self.sock_mut())
    }
}

struct Address {
    host: PyStringRef,
    port: u16,
}

impl ToSocketAddrs for Address {
    type Iter = std::vec::IntoIter<SocketAddr>;
    fn to_socket_addrs(&self) -> io::Result<Self::Iter> {
        (self.host.borrow_value(), self.port).to_socket_addrs()
    }
}

impl TryFromObject for Address {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        let tuple = PyTupleRef::try_from_object(vm, obj)?;
        if tuple.borrow_value().len() != 2 {
            Err(vm.new_type_error("Address tuple should have only 2 values".to_owned()))
        } else {
            let host = PyStringRef::try_from_object(vm, tuple.borrow_value()[0].clone())?;
            let host = if host.borrow_value().is_empty() {
                PyString::from("0.0.0.0").into_ref(vm)
            } else {
                host
            };
            let port = u16::try_from_object(vm, tuple.borrow_value()[1].clone())?;
            Ok(Address { host, port })
        }
    }
}

type AddrTuple = (String, u16);

fn get_addr_tuple<A: Into<socket2::SockAddr>>(addr: A) -> AddrTuple {
    let addr = addr.into();
    if let Some(addr) = addr.as_inet() {
        (addr.ip().to_string(), addr.port())
    } else if let Some(addr) = addr.as_inet6() {
        (addr.ip().to_string(), addr.port())
    } else {
        (String::new(), 0)
    }
}

fn socket_gethostname(vm: &VirtualMachine) -> PyResult {
    gethostname()
        .into_string()
        .map(|hostname| vm.ctx.new_str(hostname))
        .map_err(|err| vm.new_os_error(err.into_string().unwrap()))
}

#[cfg(all(unix, not(target_os = "redox")))]
fn socket_sethostname(hostname: PyStringRef, vm: &VirtualMachine) -> PyResult<()> {
    sethostname(hostname.borrow_value()).map_err(|err| err.into_pyexception(vm))
}

fn socket_inet_aton(ip_string: PyStringRef, vm: &VirtualMachine) -> PyResult {
    ip_string
        .borrow_value()
        .parse::<Ipv4Addr>()
        .map(|ip_addr| vm.ctx.new_bytes(ip_addr.octets().to_vec()))
        .map_err(|_| vm.new_os_error("illegal IP address string passed to inet_aton".to_owned()))
}

fn socket_inet_ntoa(packed_ip: PyBytesRef, vm: &VirtualMachine) -> PyResult {
    if packed_ip.len() != 4 {
        return Err(vm.new_os_error("packed IP wrong length for inet_ntoa".to_owned()));
    }
    let ip_num = BigEndian::read_u32(&packed_ip);
    Ok(vm.ctx.new_str(Ipv4Addr::from(ip_num).to_string()))
}

#[derive(FromArgs)]
struct GAIOptions {
    #[pyarg(positional_only)]
    host: Option<PyStringRef>,
    #[pyarg(positional_only)]
    port: Option<Either<PyStringRef, i32>>,

    #[pyarg(positional_only, default = "0")]
    family: i32,
    #[pyarg(positional_only, default = "0")]
    ty: i32,
    #[pyarg(positional_only, default = "0")]
    proto: i32,
    #[pyarg(positional_only, default = "0")]
    flags: i32,
}

#[cfg(not(target_os = "redox"))]
fn socket_getaddrinfo(opts: GAIOptions, vm: &VirtualMachine) -> PyResult {
    let hints = dns_lookup::AddrInfoHints {
        socktype: opts.ty,
        protocol: opts.proto,
        address: opts.family,
        flags: opts.flags,
    };

    let host = opts.host.as_ref().map(|s| s.borrow_value());
    let port = opts.port.as_ref().map(|p| -> std::borrow::Cow<str> {
        match p {
            Either::A(ref s) => s.borrow_value().into(),
            Either::B(i) => i.to_string().into(),
        }
    });
    let port = port.as_ref().map(|p| p.as_ref());

    let addrs = dns_lookup::getaddrinfo(host, port, Some(hints)).map_err(|err| {
        let error_type = vm.class("_socket", "gaierror");
        vm.new_exception_msg(error_type, io::Error::from(err).to_string())
    })?;

    let list = addrs
        .map(|ai| {
            ai.map(|ai| {
                vm.ctx.new_tuple(vec![
                    vm.ctx.new_int(ai.address),
                    vm.ctx.new_int(ai.socktype),
                    vm.ctx.new_int(ai.protocol),
                    match ai.canonname {
                        Some(s) => vm.ctx.new_str(s),
                        None => vm.get_none(),
                    },
                    get_addr_tuple(ai.sockaddr).into_pyobject(vm),
                ])
            })
        })
        .collect::<io::Result<Vec<_>>>()
        .map_err(|e| convert_sock_error(vm, e))?;
    Ok(vm.ctx.new_list(list))
}

#[cfg(not(target_os = "redox"))]
fn socket_gethostbyaddr(
    addr: PyStringRef,
    vm: &VirtualMachine,
) -> PyResult<(String, PyObjectRef, PyObjectRef)> {
    // TODO: figure out how to do this properly
    let ai = dns_lookup::getaddrinfo(Some(addr.borrow_value()), None, None)
        .map_err(|e| convert_sock_error(vm, e.into()))?
        .next()
        .unwrap()
        .map_err(|e| convert_sock_error(vm, e))?;
    let (hostname, _) =
        dns_lookup::getnameinfo(&ai.sockaddr, 0).map_err(|e| convert_sock_error(vm, e.into()))?;
    Ok((
        hostname,
        vm.ctx.new_list(vec![]),
        vm.ctx
            .new_list(vec![vm.ctx.new_str(ai.sockaddr.ip().to_string())]),
    ))
}

fn get_addr<T, I>(vm: &VirtualMachine, addr: T) -> PyResult<socket2::SockAddr>
where
    T: ToSocketAddrs<Iter = I>,
    I: ExactSizeIterator<Item = SocketAddr>,
{
    match addr.to_socket_addrs() {
        Ok(mut sock_addrs) => {
            if sock_addrs.len() == 0 {
                let error_type = vm.class("_socket", "gaierror");
                Err(vm.new_exception_msg(
                    error_type,
                    "nodename nor servname provided, or not known".to_owned(),
                ))
            } else {
                Ok(sock_addrs.next().unwrap().into())
            }
        }
        Err(e) => {
            let error_type = vm.class("_socket", "gaierror");
            Err(vm.new_exception_msg(error_type, e.to_string()))
        }
    }
}

fn sock_fileno(sock: &Socket) -> RawSocket {
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        sock.as_raw_fd()
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawSocket;
        sock.as_raw_socket()
    }
}
fn into_sock_fileno(sock: Socket) -> RawSocket {
    #[cfg(unix)]
    {
        use std::os::unix::io::IntoRawFd;
        sock.into_raw_fd()
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::IntoRawSocket;
        sock.into_raw_socket()
    }
}

fn invalid_sock() -> Socket {
    #[cfg(unix)]
    {
        use std::os::unix::io::FromRawFd;
        unsafe { Socket::from_raw_fd(-1) }
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::FromRawSocket;
        unsafe { Socket::from_raw_socket(winapi::um::winsock2::INVALID_SOCKET as RawSocket) }
    }
}

fn convert_sock_error(vm: &VirtualMachine, err: io::Error) -> PyBaseExceptionRef {
    if err.kind() == io::ErrorKind::TimedOut {
        let socket_timeout = vm.class("_socket", "timeout");
        vm.new_exception_msg(socket_timeout, "Timed out".to_owned())
    } else {
        err.into_pyexception(vm)
    }
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;
    let socket_timeout = ctx.new_class("socket.timeout", vm.ctx.exceptions.os_error.clone());
    let socket_gaierror = ctx.new_class("socket.gaierror", vm.ctx.exceptions.os_error.clone());

    let module = py_module!(vm, "_socket", {
        "socket" => PySocket::make_class(ctx),
        "error" => ctx.exceptions.os_error.clone(),
        "timeout" => socket_timeout,
        "gaierror" => socket_gaierror,
        "inet_aton" => ctx.new_function(socket_inet_aton),
        "inet_ntoa" => ctx.new_function(socket_inet_ntoa),
        "gethostname" => ctx.new_function(socket_gethostname),
        "htonl" => ctx.new_function(u32::to_be),
        "htons" => ctx.new_function(u16::to_be),
        "ntohl" => ctx.new_function(u32::from_be),
        "ntohs" => ctx.new_function(u16::from_be),
        "getdefaulttimeout" => ctx.new_function(|vm: &VirtualMachine| vm.get_none()),
        "has_ipv6" => ctx.new_bool(false),
        // constants
        "AF_UNSPEC" => ctx.new_int(0),
        "AF_INET" => ctx.new_int(c::AF_INET),
        "AF_INET6" => ctx.new_int(c::AF_INET6),
        "SOCK_STREAM" => ctx.new_int(c::SOCK_STREAM),
        "SOCK_DGRAM" => ctx.new_int(c::SOCK_DGRAM),
        "SHUT_RD" => ctx.new_int(c::SHUT_RD),
        "SHUT_WR" => ctx.new_int(c::SHUT_WR),
        "SHUT_RDWR" => ctx.new_int(c::SHUT_RDWR),
        "MSG_PEEK" => ctx.new_int(c::MSG_PEEK),
        "MSG_OOB" => ctx.new_int(c::MSG_OOB),
        "MSG_WAITALL" => ctx.new_int(c::MSG_WAITALL),
        "IPPROTO_TCP" => ctx.new_int(c::IPPROTO_TCP),
        "IPPROTO_UDP" => ctx.new_int(c::IPPROTO_UDP),
        "IPPROTO_IP" => ctx.new_int(c::IPPROTO_IP),
        "IPPROTO_IPIP" => ctx.new_int(c::IPPROTO_IP),
        "IPPROTO_IPV6" => ctx.new_int(c::IPPROTO_IPV6),
        "SOL_SOCKET" => ctx.new_int(c::SOL_SOCKET),
        "SO_REUSEADDR" => ctx.new_int(c::SO_REUSEADDR),
        "SO_TYPE" => ctx.new_int(c::SO_TYPE),
        "SO_BROADCAST" => ctx.new_int(c::SO_BROADCAST),
        "TCP_NODELAY" => ctx.new_int(c::TCP_NODELAY),
        "AI_ALL" => ctx.new_int(c::AI_ALL),
        "AI_PASSIVE" => ctx.new_int(c::AI_PASSIVE),
    });

    #[cfg(not(target_os = "redox"))]
    extend_module!(vm, module, {
        "getaddrinfo" => ctx.new_function(socket_getaddrinfo),
        "gethostbyaddr" => ctx.new_function(socket_gethostbyaddr),
    });

    extend_module_platform_specific(vm, &module);

    module
}

#[cfg(not(unix))]
fn extend_module_platform_specific(_vm: &VirtualMachine, _module: &PyObjectRef) {}

#[cfg(unix)]
fn extend_module_platform_specific(vm: &VirtualMachine, module: &PyObjectRef) {
    let ctx = &vm.ctx;

    #[cfg(not(target_os = "redox"))]
    extend_module!(vm, module, {
        "sethostname" => ctx.new_function(socket_sethostname),
    });
}
