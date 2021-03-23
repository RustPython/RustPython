use crossbeam_utils::atomic::AtomicCell;
use gethostname::gethostname;
#[cfg(all(unix, not(target_os = "redox")))]
use nix::unistd::sethostname;
use socket2::{Domain, Protocol, Socket, Type as SocketType};
use std::convert::TryFrom;
use std::io;
use std::mem::MaybeUninit;
use std::net::{Ipv4Addr, Ipv6Addr, Shutdown, SocketAddr, ToSocketAddrs};
use std::time::{Duration, Instant};

use crate::builtins::bytes::PyBytesRef;
use crate::builtins::pystr::{PyStr, PyStrRef};
use crate::builtins::pytype::PyTypeRef;
use crate::builtins::tuple::PyTupleRef;
use crate::byteslike::{PyBytesLike, PyRwBytesLike};
use crate::common::lock::{PyRwLock, PyRwLockReadGuard, PyRwLockWriteGuard};
use crate::exceptions::{IntoPyException, PyBaseExceptionRef};
use crate::function::{FuncArgs, OptionalArg};
use crate::pyobject::{
    BorrowValue, Either, IntoPyObject, PyClassImpl, PyObjectRef, PyRef, PyResult, PyValue,
    StaticType, TryFromObject, TypeProtocol,
};
use crate::VirtualMachine;

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
    pub const AI_NUMERICHOST: c_int = 0x0004;
    #[cfg(target_os = "redox")]
    pub const AI_ALL: c_int = 0x10;
    #[cfg(target_os = "redox")]
    pub const AI_ADDRCONFIG: c_int = 0x0020;
    #[cfg(target_os = "redox")]
    pub const AI_NUMERICSERV: c_int = 0x0400;
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
        SOCK_RDM, SOCK_STREAM, SOL_SOCKET, SO_BROADCAST, SO_ERROR, SO_OOBINLINE, SO_REUSEADDR,
        SO_TYPE, *,
    };
}

#[pyclass(module = "socket", name = "socket")]
#[derive(Debug)]
pub struct PySocket {
    kind: AtomicCell<i32>,
    family: AtomicCell<i32>,
    proto: AtomicCell<i32>,
    pub(crate) timeout: AtomicCell<f64>,
    sock: PyRwLock<Socket>,
}

impl PyValue for PySocket {
    fn class(_vm: &VirtualMachine) -> &PyTypeRef {
        Self::static_type()
    }
}

pub type PySocketRef = PyRef<PySocket>;

#[pyimpl(flags(BASETYPE))]
impl PySocket {
    pub fn sock(&self) -> PyRwLockReadGuard<'_, Socket> {
        self.sock.read()
    }

    fn sock_mut(&self) -> PyRwLockWriteGuard<'_, Socket> {
        self.sock.write()
    }

    #[pyslot]
    fn tp_new(cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        PySocket {
            kind: AtomicCell::default(),
            family: AtomicCell::default(),
            proto: AtomicCell::default(),
            timeout: AtomicCell::new(-1.0),
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

    fn sock_op<F, R>(&self, vm: &VirtualMachine, select: SelectKind, f: F) -> PyResult<R>
    where
        F: FnMut() -> io::Result<R>,
    {
        let timeout = self.timeout.load();
        let timeout = if timeout > 0.0 {
            Some(Duration::from_secs_f64(timeout))
        } else {
            None
        };
        self.sock_op_timeout(vm, select, timeout, f)
    }

    fn sock_op_timeout<F, R>(
        &self,
        vm: &VirtualMachine,
        select: SelectKind,
        timeout: Option<Duration>,
        mut f: F,
    ) -> PyResult<R>
    where
        F: FnMut() -> io::Result<R>,
    {
        let deadline = timeout.map(Deadline::new);

        loop {
            if deadline.is_some() || matches!(select, SelectKind::Connect) {
                let interval = deadline.as_ref().map(|d| d.time_until(vm)).transpose()?;
                let res = sock_select(&self.sock(), select, interval);
                match res {
                    Ok(true) => return Err(timeout_error(vm)),
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {
                        vm.check_signals()?;
                        continue;
                    }
                    Err(e) => return Err(convert_sock_error(vm, e)),
                    Ok(false) => {} // no timeout, continue as normal
                }
            }

            let err = loop {
                // loop on interrupt
                match f() {
                    Ok(x) => return Ok(x),
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => vm.check_signals()?,
                    Err(e) => break e,
                }
            };
            if timeout.is_some() && err.kind() == io::ErrorKind::WouldBlock {
                continue;
            }
            return Err(convert_sock_error(vm, err));
        }
    }

    fn extract_address(
        &self,
        addr: PyObjectRef,
        caller: &str,
        vm: &VirtualMachine,
    ) -> PyResult<socket2::SockAddr> {
        let family = self.family.load();
        match family {
            c::AF_INET => {
                let addr = Address::try_from_object(vm, addr)?;
                let addr4 = get_addr(vm, addr, |sa| match sa {
                    SocketAddr::V4(v4) => Some(v4),
                    _ => None,
                })?;
                Ok(addr4.into())
            }
            c::AF_INET6 => {
                let tuple: PyTupleRef = addr.downcast().map_err(|obj| {
                    vm.new_type_error(format!(
                        "{}(): AF_INET6 address must be tuple, not {}",
                        caller,
                        obj.class().name
                    ))
                })?;
                let tuple = tuple.borrow_value();
                match tuple.len() {
                    2 | 3 | 4 => {}
                    _ => {
                        return Err(vm.new_type_error(
                            "AF_INET6 address must be a tuple (host, port[, flowinfo[, scopeid]])"
                                .to_owned(),
                        ))
                    }
                }
                let addr = Address::from_tuple(tuple, vm)?;
                let flowinfo = tuple
                    .get(2)
                    .map(|obj| u32::try_from_object(vm, obj.clone()))
                    .transpose()?;
                let scopeid = tuple
                    .get(3)
                    .map(|obj| u32::try_from_object(vm, obj.clone()))
                    .transpose()?;
                let mut addr6 = get_addr(vm, addr, |sa| match sa {
                    SocketAddr::V6(v6) => Some(v6),
                    _ => None,
                })?;
                if let Some(fi) = flowinfo {
                    addr6.set_flowinfo(fi)
                }
                if let Some(si) = scopeid {
                    addr6.set_scope_id(si)
                }
                Ok(addr6.into())
            }
            _ => Err(vm.new_os_error(format!("{}(): bad family", caller))),
        }
    }

    #[pymethod]
    fn connect(&self, address: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let sock_addr = self.extract_address(address, "connect", vm)?;

        let err = match self.sock().connect(&sock_addr) {
            Ok(()) => return Ok(()),
            Err(e) => e,
        };

        let wait_connect = if err.kind() == io::ErrorKind::Interrupted {
            vm.check_signals()?;
            self.timeout.load() != 0.0
        } else {
            self.timeout.load() > 0.0 && err.raw_os_error() == Some(libc::EINPROGRESS)
        };

        if wait_connect {
            // basically, connect() is async, and it registers an "error" on the socket when it's
            // done connecting. SelectKind::Connect fills the errorfds fd_set, so if we wake up
            // from poll and the error is EISCONN then we know that the connect is done
            self.sock_op(vm, SelectKind::Connect, || {
                let sock = self.sock();
                let err = sock.take_error()?;
                match err {
                    Some(e) if e.raw_os_error() == Some(libc::EISCONN) => Ok(()),
                    Some(e) => Err(e),
                    // TODO: is this accurate?
                    None => Ok(()),
                }
            })
        } else {
            Err(convert_sock_error(vm, err))
        }
    }

    #[pymethod]
    fn bind(&self, address: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let sock_addr = self.extract_address(address, "bind", vm)?;
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
    fn _accept(&self, vm: &VirtualMachine) -> PyResult<(RawSocket, PyObjectRef)> {
        let (sock, addr) = self.sock_op(vm, SelectKind::Read, || self.sock().accept())?;
        let fd = into_sock_fileno(sock);
        Ok((fd, get_addr_tuple(addr, vm)))
    }

    #[pymethod]
    fn recv(
        &self,
        bufsize: usize,
        flags: OptionalArg<i32>,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<u8>> {
        let flags = flags.unwrap_or(0);
        let mut buffer = Vec::with_capacity(bufsize);
        let sock = self.sock();
        let n = self.sock_op(vm, SelectKind::Read, || {
            sock.recv_with_flags(spare_capacity_mut(&mut buffer), flags)
        })?;
        unsafe { buffer.set_len(n) };
        Ok(buffer)
    }

    #[pymethod]
    fn recv_into(
        &self,
        buf: PyRwBytesLike,
        flags: OptionalArg<i32>,
        vm: &VirtualMachine,
    ) -> PyResult<usize> {
        let flags = flags.unwrap_or(0);
        let sock = self.sock();
        self.sock_op(vm, SelectKind::Read, || {
            buf.with_ref(|buf| sock.recv_with_flags(slice_as_uninit(buf), flags))
        })
    }

    #[pymethod]
    fn recvfrom(
        &self,
        bufsize: usize,
        flags: OptionalArg<i32>,
        vm: &VirtualMachine,
    ) -> PyResult<(Vec<u8>, PyObjectRef)> {
        let flags = flags.unwrap_or(0);
        let mut buffer = Vec::with_capacity(bufsize);
        let (n, addr) = self.sock_op(vm, SelectKind::Read, || {
            self.sock()
                .recv_from_with_flags(spare_capacity_mut(&mut buffer), flags)
        })?;
        unsafe { buffer.set_len(n) };
        Ok((buffer, get_addr_tuple(addr, vm)))
    }

    #[pymethod]
    fn send(
        &self,
        bytes: PyBytesLike,
        flags: OptionalArg<i32>,
        vm: &VirtualMachine,
    ) -> PyResult<usize> {
        let flags = flags.unwrap_or(0);
        self.sock_op(vm, SelectKind::Write, || {
            bytes.with_ref(|b| self.sock().send_with_flags(b, flags))
        })
    }

    #[pymethod]
    fn sendall(
        &self,
        bytes: PyBytesLike,
        flags: OptionalArg<i32>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let flags = flags.unwrap_or(0);

        let timeout = self.timeout.load();
        let timeout = if timeout > 0.0 {
            Some(Duration::from_secs_f64(timeout))
        } else {
            None
        };

        let deadline = timeout.map(Deadline::new);

        let buf_len = bytes.len();
        let mut buf_offset = 0;
        // now we have like 3 layers of interrupt loop :)
        while buf_offset < buf_len {
            let interval = deadline.as_ref().map(|d| d.time_until(vm)).transpose()?;
            self.sock_op_timeout(vm, SelectKind::Write, interval, || {
                bytes.with_ref(|b| {
                    let subbuf = &b[buf_offset..];
                    buf_offset += self.sock().send_with_flags(subbuf, flags)?;
                    Ok(())
                })
            })?;
            vm.check_signals()?;
        }
        Ok(())
    }

    #[pymethod]
    fn sendto(
        &self,
        bytes: PyBytesLike,
        address: PyObjectRef,
        flags: OptionalArg<i32>,
        vm: &VirtualMachine,
    ) -> PyResult<usize> {
        let flags = flags.unwrap_or(0);
        let addr = self.extract_address(address, "sendto", vm)?;
        self.sock_op(vm, SelectKind::Write, || {
            bytes.with_ref(|b| self.sock().send_to_with_flags(b, &addr, flags))
        })
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
    fn getsockname(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let addr = self
            .sock()
            .local_addr()
            .map_err(|err| convert_sock_error(vm, err))?;

        Ok(get_addr_tuple(addr, vm))
    }
    #[pymethod]
    fn getpeername(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let addr = self
            .sock()
            .peer_addr()
            .map_err(|err| convert_sock_error(vm, err))?;

        Ok(get_addr_tuple(addr, vm))
    }

    #[pymethod]
    fn gettimeout(&self) -> Option<f64> {
        let timeout = self.timeout.load();
        if timeout >= 0.0 {
            Some(timeout)
        } else {
            None
        }
    }

    #[pymethod]
    fn setblocking(&self, block: bool, vm: &VirtualMachine) -> PyResult<()> {
        self.timeout.store(if block { -1.0 } else { 0.0 });
        self.sock()
            .set_nonblocking(!block)
            .map_err(|err| convert_sock_error(vm, err))
    }

    #[pymethod]
    fn getblocking(&self) -> bool {
        self.timeout.load() != 0.0
    }

    #[pymethod]
    fn settimeout(&self, timeout: Option<Duration>, vm: &VirtualMachine) -> PyResult<()> {
        // timeout is None: blocking, no timeout
        // timeout is 0: non-blocking, no timeout
        // otherwise: timeout is timeout, don't change blocking
        let (block, timeout) = match timeout {
            None => (true, -1.0),
            Some(d) if d == Duration::from_secs(0) => (false, 0.0),
            Some(d) => (true, d.as_secs_f64()),
        };
        self.timeout.store(timeout);
        self.sock()
            .set_nonblocking(!block)
            .map_err(|err| convert_sock_error(vm, err))
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
        <&Socket as io::Read>::read(&mut &*self.sock(), buf)
    }
}
impl io::Write for PySocketRef {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        <&Socket as io::Write>::write(&mut &*self.sock(), buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        <&Socket as io::Write>::flush(&mut &*self.sock())
    }
}

struct Address {
    host: PyStrRef,
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
            Self::from_tuple(tuple.borrow_value(), vm)
        }
    }
}

impl Address {
    fn from_tuple(tuple: &[PyObjectRef], vm: &VirtualMachine) -> PyResult<Self> {
        let host = PyStrRef::try_from_object(vm, tuple[0].clone())?;
        let host = if host.borrow_value().is_empty() {
            PyStr::from("0.0.0.0").into_ref(vm)
        } else {
            host
        };
        let port = u16::try_from_object(vm, tuple[1].clone())?;
        Ok(Address { host, port })
    }
}

fn get_addr_tuple<A: Into<socket2::SockAddr>>(addr: A, vm: &VirtualMachine) -> PyObjectRef {
    let addr = addr.into();
    match addr.as_socket() {
        Some(SocketAddr::V4(addr)) => (addr.ip().to_string(), addr.port()).into_pyobject(vm),
        Some(SocketAddr::V6(addr)) => (
            addr.ip().to_string(),
            addr.port(),
            addr.flowinfo(),
            addr.scope_id(),
        )
            .into_pyobject(vm),
        // TODO: support AF_UNIX et al
        None => (String::new(), 0).into_pyobject(vm),
    }
}

fn _socket_gethostname(vm: &VirtualMachine) -> PyResult {
    gethostname()
        .into_string()
        .map(|hostname| vm.ctx.new_str(hostname))
        .map_err(|err| vm.new_os_error(err.into_string().unwrap()))
}

#[cfg(all(unix, not(target_os = "redox")))]
fn _socket_sethostname(hostname: PyStrRef, vm: &VirtualMachine) -> PyResult<()> {
    sethostname(hostname.borrow_value()).map_err(|err| err.into_pyexception(vm))
}

fn _socket_inet_aton(ip_string: PyStrRef, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
    ip_string
        .borrow_value()
        .parse::<Ipv4Addr>()
        .map(|ip_addr| Vec::<u8>::from(ip_addr.octets()))
        .map_err(|_| vm.new_os_error("illegal IP address string passed to inet_aton".to_owned()))
}

fn _socket_inet_ntoa(packed_ip: PyBytesRef, vm: &VirtualMachine) -> PyResult {
    let packed_ip = <&[u8; 4]>::try_from(&**packed_ip)
        .map_err(|_| vm.new_os_error("packed IP wrong length for inet_ntoa".to_owned()))?;
    Ok(vm.ctx.new_str(Ipv4Addr::from(*packed_ip).to_string()))
}

fn _socket_getservbyname(
    servicename: PyStrRef,
    protocolname: OptionalArg<PyStrRef>,
    vm: &VirtualMachine,
) -> PyResult {
    use std::ffi::CString;
    let cstr_name = CString::new(servicename.borrow_value())
        .map_err(|_| vm.new_value_error("embedded null character".to_owned()))?;
    let protocolname = protocolname.as_ref().map_or("", |s| s.borrow_value());
    let cstr_proto = CString::new(protocolname)
        .map_err(|_| vm.new_value_error("embedded null character".to_owned()))?;
    let serv = unsafe { c::getservbyname(cstr_name.as_ptr(), cstr_proto.as_ptr()) };
    if serv.is_null() {
        return Err(vm.new_os_error("service/proto not found".to_owned()));
    }
    let port = unsafe { (*serv).s_port };
    Ok(vm.ctx.new_int(u16::from_be(port as u16)))
}

// TODO: use `Vec::spare_capacity_mut` once stable.
fn spare_capacity_mut<T>(v: &mut Vec<T>) -> &mut [MaybeUninit<T>] {
    let (len, cap) = (v.len(), v.capacity());
    unsafe {
        std::slice::from_raw_parts_mut(v.as_mut_ptr().add(len) as *mut MaybeUninit<T>, cap - len)
    }
}
fn slice_as_uninit<T>(v: &mut [T]) -> &mut [MaybeUninit<T>] {
    unsafe { &mut *(v as *mut [T] as *mut [MaybeUninit<T>]) }
}

#[derive(Copy, Clone)]
enum SelectKind {
    Read,
    Write,
    Connect,
}

/// returns true if timed out
fn sock_select(sock: &Socket, kind: SelectKind, interval: Option<Duration>) -> io::Result<bool> {
    let fd = sock_fileno(sock);
    #[cfg(unix)]
    {
        let mut pollfd = libc::pollfd {
            fd,
            events: match kind {
                SelectKind::Read => libc::POLLIN,
                SelectKind::Write => libc::POLLOUT,
                SelectKind::Connect => libc::POLLOUT | libc::POLLERR,
            },
            revents: 0,
        };
        let timeout = match interval {
            Some(d) => d.as_millis() as _,
            None => -1,
        };
        let ret = unsafe { libc::poll(&mut pollfd, 1, timeout) };
        if ret < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(ret == 0)
        }
    }
    #[cfg(windows)]
    {
        use crate::stdlib::select;

        let mut reads = select::FdSet::new();
        let mut writes = select::FdSet::new();
        let mut errs = select::FdSet::new();

        let fd = fd as usize;
        match kind {
            SelectKind::Read => reads.insert(fd),
            SelectKind::Write => writes.insert(fd),
            SelectKind::Connect => {
                writes.insert(fd);
                errs.insert(fd);
            }
        }

        let mut interval = interval.map(|dur| select::timeval {
            tv_sec: dur.as_secs() as _,
            tv_usec: dur.subsec_micros() as _,
        });

        select::select(
            fd as i32 + 1,
            &mut reads,
            &mut writes,
            &mut errs,
            interval.as_mut(),
        )
        .map(|ret| ret == 0)
    }
}

#[derive(FromArgs)]
struct GAIOptions {
    #[pyarg(positional)]
    host: Option<PyStrRef>,
    #[pyarg(positional)]
    port: Option<Either<PyStrRef, i32>>,

    #[pyarg(positional, default = "0")]
    family: i32,
    #[pyarg(positional, default = "0")]
    ty: i32,
    #[pyarg(positional, default = "0")]
    proto: i32,
    #[pyarg(positional, default = "0")]
    flags: i32,
}

#[cfg(not(target_os = "redox"))]
fn _socket_getaddrinfo(opts: GAIOptions, vm: &VirtualMachine) -> PyResult {
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
        let error_type = GAI_ERROR.get().unwrap().clone();
        let code = err.error_num();
        let strerr = {
            #[cfg(unix)]
            {
                let x = unsafe { libc::gai_strerror(code) };
                if x.is_null() {
                    io::Error::from(err).to_string()
                } else {
                    unsafe { std::ffi::CStr::from_ptr(x) }
                        .to_string_lossy()
                        .into_owned()
                }
            }
            #[cfg(not(unix))]
            {
                io::Error::from(err).to_string()
            }
        };
        vm.new_exception(
            error_type,
            vec![vm.ctx.new_int(code), vm.ctx.new_str(strerr)],
        )
    })?;

    let list = addrs
        .map(|ai| {
            ai.map(|ai| {
                vm.ctx.new_tuple(vec![
                    vm.ctx.new_int(ai.address),
                    vm.ctx.new_int(ai.socktype),
                    vm.ctx.new_int(ai.protocol),
                    ai.canonname.into_pyobject(vm),
                    get_addr_tuple(ai.sockaddr, vm),
                ])
            })
        })
        .collect::<io::Result<Vec<_>>>()
        .map_err(|e| convert_sock_error(vm, e))?;
    Ok(vm.ctx.new_list(list))
}

#[cfg(not(target_os = "redox"))]
fn _socket_gethostbyaddr(
    addr: PyStrRef,
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

#[cfg(not(target_os = "redox"))]
fn _socket_gethostbyname(name: PyStrRef, vm: &VirtualMachine) -> PyResult<String> {
    match _socket_gethostbyaddr(name, vm) {
        Ok((_, _, hosts)) => {
            let lst = vm.extract_elements::<PyStrRef>(&hosts)?;
            Ok(lst.get(0).unwrap().to_string())
        }
        Err(_) => {
            let error_type = GAI_ERROR.get().unwrap().clone();
            Err(vm.new_exception_msg(
                error_type,
                "nodename nor servname provided, or not known".to_owned(),
            ))
        }
    }
}

fn _socket_inet_pton(af_inet: i32, ip_string: PyStrRef, vm: &VirtualMachine) -> PyResult {
    match af_inet {
        c::AF_INET => ip_string
            .borrow_value()
            .parse::<Ipv4Addr>()
            .map(|ip_addr| vm.ctx.new_bytes(ip_addr.octets().to_vec()))
            .map_err(|_| {
                vm.new_os_error("illegal IP address string passed to inet_pton".to_owned())
            }),
        c::AF_INET6 => ip_string
            .borrow_value()
            .parse::<Ipv6Addr>()
            .map(|ip_addr| vm.ctx.new_bytes(ip_addr.octets().to_vec()))
            .map_err(|_| {
                vm.new_os_error("illegal IP address string passed to inet_pton".to_owned())
            }),
        _ => Err(vm.new_os_error("Address family not supported by protocol".to_owned())),
    }
}

fn _socket_inet_ntop(af_inet: i32, packed_ip: PyBytesRef, vm: &VirtualMachine) -> PyResult<String> {
    match af_inet {
        c::AF_INET => {
            let packed_ip = <&[u8; 4]>::try_from(&**packed_ip).map_err(|_| {
                vm.new_value_error("invalid length of packed IP address string".to_owned())
            })?;
            Ok(Ipv4Addr::from(*packed_ip).to_string())
        }
        c::AF_INET6 => {
            let packed_ip = <&[u8; 16]>::try_from(&**packed_ip).map_err(|_| {
                vm.new_value_error("invalid length of packed IP address string".to_owned())
            })?;
            Ok(get_ipv6_addr_str(Ipv6Addr::from(*packed_ip)))
        }
        _ => Err(vm.new_value_error(format!("unknown address family {}", af_inet))),
    }
}

fn _socket_getprotobyname(name: PyStrRef, vm: &VirtualMachine) -> PyResult {
    use std::ffi::CString;
    let cstr = CString::new(name.borrow_value())
        .map_err(|_| vm.new_value_error("embedded null character".to_owned()))?;
    let proto = unsafe { c::getprotobyname(cstr.as_ptr()) };
    if proto.is_null() {
        return Err(vm.new_os_error("protocol not found".to_owned()));
    }
    let num = unsafe { (*proto).p_proto };
    Ok(vm.ctx.new_int(num))
}

#[cfg(not(target_os = "redox"))]
fn _socket_getnameinfo(
    address: Address,
    flags: i32,
    vm: &VirtualMachine,
) -> PyResult<(String, String)> {
    let addr = get_addr(vm, address, Some)?;
    dns_lookup::getnameinfo(&addr, flags).map_err(|_| {
        let error_type = GAI_ERROR.get().unwrap().clone();
        vm.new_exception_msg(
            error_type,
            "nodename nor servname provided, or not known".to_owned(),
        )
    })
}

fn get_addr<R>(
    vm: &VirtualMachine,
    addr: impl ToSocketAddrs,
    filter: impl FnMut(SocketAddr) -> Option<R>,
) -> PyResult<R> {
    let mut sock_addrs = addr.to_socket_addrs().map_err(|e| {
        let error_type = GAI_ERROR.get().unwrap().clone();
        vm.new_exception_msg(error_type, e.to_string())
    })?;
    sock_addrs.find_map(filter).ok_or_else(|| {
        let error_type = GAI_ERROR.get().unwrap().clone();
        vm.new_exception_msg(
            error_type,
            "nodename nor servname provided, or not known".to_owned(),
        )
    })
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
        let socket_timeout = TIMEOUT_ERROR.get().unwrap().clone();
        vm.new_exception_msg(socket_timeout, "Timed out".to_owned())
    } else {
        err.into_pyexception(vm)
    }
}

fn timeout_error(vm: &VirtualMachine) -> PyBaseExceptionRef {
    vm.new_exception_msg(TIMEOUT_ERROR.get().unwrap().clone(), "timed out".to_owned())
}

fn get_ipv6_addr_str(ipv6: Ipv6Addr) -> String {
    match ipv6.to_ipv4() {
        Some(v4) if matches!(v4.octets(), [0, 0, _, _]) => format!("::{:x}", u32::from(v4)),
        _ => ipv6.to_string(),
    }
}

struct Deadline {
    deadline: Instant,
}

impl Deadline {
    fn new(timeout: Duration) -> Self {
        Self {
            deadline: Instant::now() + timeout,
        }
    }
    fn time_until(&self, vm: &VirtualMachine) -> PyResult<Duration> {
        self.deadline
            .checked_duration_since(Instant::now())
            // past the deadline already
            .ok_or_else(|| timeout_error(vm))
    }
}

rustpython_common::static_cell! {
    static TIMEOUT_ERROR: PyTypeRef;
    static GAI_ERROR: PyTypeRef;
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;
    let socket_timeout = TIMEOUT_ERROR
        .get_or_init(|| {
            ctx.new_class(
                "socket.timeout",
                &vm.ctx.exceptions.os_error,
                Default::default(),
            )
        })
        .clone();
    let socket_gaierror = GAI_ERROR
        .get_or_init(|| {
            ctx.new_class(
                "socket.gaierror",
                &vm.ctx.exceptions.os_error,
                Default::default(),
            )
        })
        .clone();

    let module = py_module!(vm, "_socket", {
        "socket" => PySocket::make_class(ctx),
        "error" => ctx.exceptions.os_error.clone(),
        "timeout" => socket_timeout,
        "gaierror" => socket_gaierror,
        "inet_aton" => named_function!(ctx, _socket, inet_aton),
        "inet_ntoa" => named_function!(ctx, _socket, inet_ntoa),
        "gethostname" => named_function!(ctx, _socket, gethostname),
        "htonl" => ctx.new_function("htonl", u32::to_be),
        "htons" => ctx.new_function("htons", u16::to_be),
        "ntohl" => ctx.new_function("ntohl", u32::from_be),
        "ntohs" => ctx.new_function("ntohs", u16::from_be),
        "getdefaulttimeout" => ctx.new_function("getdefaulttimeout", |vm: &VirtualMachine| vm.ctx.none()),
        "has_ipv6" => ctx.new_bool(false),
        "inet_pton" => named_function!(ctx, _socket, inet_pton),
        "inet_ntop" => named_function!(ctx, _socket, inet_ntop),
        "getprotobyname" => named_function!(ctx, _socket, getprotobyname),
        "getservbyname" => named_function!(ctx, _socket, getservbyname),
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
        "SO_OOBINLINE" => ctx.new_int(c::SO_OOBINLINE),
        "SO_ERROR" => ctx.new_int(c::SO_ERROR),
        "TCP_NODELAY" => ctx.new_int(c::TCP_NODELAY),
        "NI_NAMEREQD" => ctx.new_int(c::NI_NAMEREQD),
        "NI_NOFQDN" => ctx.new_int(c::NI_NOFQDN),
        "NI_NUMERICHOST" => ctx.new_int(c::NI_NUMERICHOST),
        "NI_NUMERICSERV" => ctx.new_int(c::NI_NUMERICSERV),
    });

    #[cfg(not(target_os = "freebsd"))]
    extend_module!(vm, module, {
        "AI_PASSIVE" => ctx.new_int(c::AI_PASSIVE),
        "AI_NUMERICHOST" => ctx.new_int(c::AI_NUMERICHOST),
        "AI_ALL" => ctx.new_int(c::AI_ALL),
        "AI_ADDRCONFIG" => ctx.new_int(c::AI_ADDRCONFIG),
        "AI_NUMERICSERV" => ctx.new_int(c::AI_NUMERICSERV),
    });

    #[cfg(not(windows))]
    extend_module!(vm, module, {
        "SO_REUSEPORT" => ctx.new_int(c::SO_REUSEPORT),
    });

    #[cfg(not(target_os = "redox"))]
    extend_module!(vm, module, {
        "getaddrinfo" => named_function!(ctx, _socket, getaddrinfo),
        "gethostbyaddr" => named_function!(ctx, _socket, gethostbyaddr),
        "gethostbyname" => named_function!(ctx, _socket, gethostbyname),
        "getnameinfo" => named_function!(ctx, _socket, getnameinfo),
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
        "sethostname" => named_function!(ctx, _socket, sethostname),
    });
}
