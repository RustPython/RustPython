use std::cell::{Cell, Ref, RefCell};
use std::io::{self, prelude::*};
use std::net::{Ipv4Addr, Shutdown, SocketAddr, ToSocketAddrs};
use std::time::Duration;

use byteorder::{BigEndian, ByteOrder};
use gethostname::gethostname;
#[cfg(all(unix, not(target_os = "redox")))]
use nix::unistd::sethostname;
use socket2::{Domain, Protocol, Socket, Type as SocketType};

use super::os::convert_io_error;
#[cfg(unix)]
use super::os::convert_nix_error;
use crate::function::{OptionalArg, PyFuncArgs};
use crate::obj::objbyteinner::PyBytesLike;
use crate::obj::objbytes::PyBytesRef;
use crate::obj::objstr::PyStringRef;
use crate::obj::objtuple::PyTupleRef;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{PyClassImpl, PyObjectRef, PyRef, PyResult, PyValue, TryFromObject};
use crate::vm::VirtualMachine;

#[cfg(unix)]
type RawSocket = std::os::unix::io::RawFd;
#[cfg(windows)]
type RawSocket = std::os::windows::raw::SOCKET;

#[cfg(unix)]
mod c {
    pub use libc::*;
    // TODO: open a PR to add these constants to libc; then just use libc
    #[cfg(target_os = "android")]
    pub const AI_PASSIVE: c_int = 0x00000001;
    #[cfg(target_os = "android")]
    pub const AI_CANONNAME: c_int = 0x00000002;
    #[cfg(target_os = "android")]
    pub const AI_NUMERICHOST: c_int = 0x00000004;
    #[cfg(target_os = "android")]
    pub const AI_NUMERICSERV: c_int = 0x00000008;
    #[cfg(target_os = "android")]
    pub const AI_MASK: c_int =
        AI_PASSIVE | AI_CANONNAME | AI_NUMERICHOST | AI_NUMERICSERV | AI_ADDRCONFIG;
    #[cfg(target_os = "android")]
    pub const AI_ALL: c_int = 0x00000100;
    #[cfg(target_os = "android")]
    pub const AI_V4MAPPED_CFG: c_int = 0x00000200;
    #[cfg(target_os = "android")]
    pub const AI_ADDRCONFIG: c_int = 0x00000400;
    #[cfg(target_os = "android")]
    pub const AI_V4MAPPED: c_int = 0x00000800;
    #[cfg(target_os = "android")]
    pub const AI_DEFAULT: c_int = AI_V4MAPPED_CFG | AI_ADDRCONFIG;
}
#[cfg(windows)]
mod c {
    pub use winapi::shared::ws2def::*;
    pub use winapi::um::winsock2::{
        SD_BOTH as SHUT_RDWR, SD_RECEIVE as SHUT_RD, SD_SEND as SHUT_WR, SOCK_DGRAM, SOCK_RAW,
        SOCK_RDM, SOCK_STREAM, *,
    };
}

#[pyclass]
#[derive(Debug)]
pub struct PySocket {
    kind: Cell<i32>,
    family: Cell<i32>,
    proto: Cell<i32>,
    sock: RefCell<Socket>,
}

impl PyValue for PySocket {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("_socket", "socket")
    }
}

pub type PySocketRef = PyRef<PySocket>;

#[pyimpl]
impl PySocket {
    fn sock(&self) -> Ref<Socket> {
        self.sock.borrow()
    }

    #[pyslot(new)]
    fn tp_new(cls: PyClassRef, _args: PyFuncArgs, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        PySocket {
            kind: Cell::default(),
            family: Cell::default(),
            proto: Cell::default(),
            sock: RefCell::new(invalid_sock()),
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

            self.family.set(family);
            self.kind.set(socket_kind);
            self.proto.set(proto);
            sock
        };
        self.sock.replace(sock);
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
    fn recv(&self, bufsize: usize, vm: &VirtualMachine) -> PyResult {
        let mut buffer = vec![0u8; bufsize];
        match self.sock.borrow_mut().read_exact(&mut buffer) {
            Ok(()) => Ok(vm.ctx.new_bytes(buffer)),
            Err(err) => Err(convert_sock_error(vm, err)),
        }
    }

    #[pymethod]
    fn recvfrom(&self, bufsize: usize, vm: &VirtualMachine) -> PyResult<(Vec<u8>, AddrTuple)> {
        let mut buffer = vec![0u8; bufsize];
        match self.sock().recv_from(&mut buffer) {
            Ok((_, addr)) => Ok((buffer, get_addr_tuple(addr))),
            Err(err) => Err(convert_sock_error(vm, err)),
        }
    }

    #[pymethod]
    fn send(&self, bytes: PyBytesLike, vm: &VirtualMachine) -> PyResult<usize> {
        self.sock()
            .send(bytes.to_cow().as_ref())
            .map_err(|err| convert_sock_error(vm, err))
    }

    #[pymethod]
    fn sendto(&self, bytes: PyBytesLike, address: Address, vm: &VirtualMachine) -> PyResult<()> {
        let addr = get_addr(vm, address)?;
        self.sock()
            .send_to(bytes.to_cow().as_ref(), &addr)
            .map_err(|err| convert_sock_error(vm, err))?;
        Ok(())
    }

    #[pymethod]
    fn close(&self, _vm: &VirtualMachine) {
        self.sock.replace(invalid_sock());
    }

    #[pymethod]
    fn fileno(&self, _vm: &VirtualMachine) -> RawSocket {
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
    fn settimeout(&self, timeout: Option<f64>, vm: &VirtualMachine) -> PyResult<()> {
        self.sock()
            .set_read_timeout(timeout.map(Duration::from_secs_f64))
            .map_err(|err| convert_sock_error(vm, err))?;
        self.sock()
            .set_write_timeout(timeout.map(Duration::from_secs_f64))
            .map_err(|err| convert_sock_error(vm, err))?;
        Ok(())
    }

    #[pymethod]
    fn shutdown(&self, how: i32, vm: &VirtualMachine) -> PyResult<()> {
        let how = match how {
            c::SHUT_RD => Shutdown::Read,
            c::SHUT_WR => Shutdown::Write,
            c::SHUT_RDWR => Shutdown::Both,
            _ => {
                return Err(
                    vm.new_value_error("`how` must be SHUT_RD, SHUT_WR, or SHUT_RDWR".to_string())
                )
            }
        };
        self.sock()
            .shutdown(how)
            .map_err(|err| convert_sock_error(vm, err))
    }

    #[pyproperty(name = "type")]
    fn kind(&self, _vm: &VirtualMachine) -> i32 {
        self.kind.get()
    }
    #[pyproperty]
    fn family(&self, _vm: &VirtualMachine) -> i32 {
        self.family.get()
    }
    #[pyproperty]
    fn proto(&self, _vm: &VirtualMachine) -> i32 {
        self.proto.get()
    }
}

struct Address {
    host: PyStringRef,
    port: u16,
}

impl ToSocketAddrs for Address {
    type Iter = std::vec::IntoIter<SocketAddr>;
    fn to_socket_addrs(&self) -> io::Result<Self::Iter> {
        (self.host.as_str(), self.port).to_socket_addrs()
    }
}

impl TryFromObject for Address {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        let tuple = PyTupleRef::try_from_object(vm, obj)?;
        if tuple.elements.len() != 2 {
            Err(vm.new_type_error("Address tuple should have only 2 values".to_string()))
        } else {
            Ok(Address {
                host: PyStringRef::try_from_object(vm, tuple.elements[0].clone())?,
                port: u16::try_from_object(vm, tuple.elements[1].clone())?,
            })
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
        .map(|hostname| vm.new_str(hostname))
        .map_err(|err| vm.new_os_error(err.into_string().unwrap()))
}

#[cfg(all(unix, not(target_os = "redox")))]
fn socket_sethostname(hostname: PyStringRef, vm: &VirtualMachine) -> PyResult<()> {
    sethostname(hostname.as_str()).map_err(|err| convert_nix_error(vm, err))
}

fn socket_inet_aton(ip_string: PyStringRef, vm: &VirtualMachine) -> PyResult {
    ip_string
        .as_str()
        .parse::<Ipv4Addr>()
        .map(|ip_addr| vm.ctx.new_bytes(ip_addr.octets().to_vec()))
        .map_err(|_| vm.new_os_error("illegal IP address string passed to inet_aton".to_string()))
}

fn socket_inet_ntoa(packed_ip: PyBytesRef, vm: &VirtualMachine) -> PyResult {
    if packed_ip.len() != 4 {
        return Err(vm.new_os_error("packed IP wrong length for inet_ntoa".to_string()));
    }
    let ip_num = BigEndian::read_u32(&packed_ip);
    Ok(vm.new_str(Ipv4Addr::from(ip_num).to_string()))
}

fn socket_htonl(host: u32, vm: &VirtualMachine) -> PyResult {
    Ok(vm.new_int(host.to_be()))
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
                Err(vm.new_exception(
                    error_type,
                    "nodename nor servname provided, or not known".to_string(),
                ))
            } else {
                Ok(sock_addrs.next().unwrap().into())
            }
        }
        Err(e) => {
            let error_type = vm.class("_socket", "gaierror");
            Err(vm.new_exception(error_type, e.to_string()))
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

fn convert_sock_error(vm: &VirtualMachine, err: io::Error) -> PyObjectRef {
    if err.kind() == io::ErrorKind::TimedOut {
        let socket_timeout = vm.class("_socket", "timeout");
        vm.new_exception(socket_timeout, "Timed out".to_string())
    } else {
        convert_io_error(vm, err)
    }
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;
    let socket_timeout = ctx.new_class("socket.timeout", vm.ctx.exceptions.os_error.clone());
    let socket_gaierror = ctx.new_class("socket.gaierror", vm.ctx.exceptions.os_error.clone());

    let module = py_module!(vm, "_socket", {
        "error" => ctx.exceptions.os_error.clone(),
        "timeout" => socket_timeout,
        "gaierror" => socket_gaierror,
        "AF_UNSPEC" => ctx.new_int(0),
        "AF_INET" => ctx.new_int(c::AF_INET),
        "AF_INET6" => ctx.new_int(c::AF_INET6),
        "SOCK_STREAM" => ctx.new_int(c::SOCK_STREAM),
        "SOCK_DGRAM" => ctx.new_int(c::SOCK_DGRAM),
        "SHUT_RD" => ctx.new_int(c::SHUT_RD),
        "SHUT_WR" => ctx.new_int(c::SHUT_WR),
        "SHUT_RDWR" => ctx.new_int(c::SHUT_RDWR),
        "MSG_OOB" => ctx.new_int(c::MSG_OOB),
        "MSG_PEEK" => ctx.new_int(c::MSG_PEEK),
        "MSG_WAITALL" => ctx.new_int(c::MSG_WAITALL),
        "AI_ALL" => ctx.new_int(c::AI_ALL),
        "AI_PASSIVE" => ctx.new_int(c::AI_PASSIVE),
        "socket" => PySocket::make_class(ctx),
        "inet_aton" => ctx.new_rustfunc(socket_inet_aton),
        "inet_ntoa" => ctx.new_rustfunc(socket_inet_ntoa),
        "gethostname" => ctx.new_rustfunc(socket_gethostname),
        "htonl" => ctx.new_rustfunc(socket_htonl),
        "getdefaulttimeout" => ctx.new_rustfunc(|vm: &VirtualMachine| vm.get_none()),
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
        "sethostname" => ctx.new_rustfunc(socket_sethostname),
    });
}
