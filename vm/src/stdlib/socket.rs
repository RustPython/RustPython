use std::cell::RefCell;
use std::io;
use std::io::Read;
use std::io::Write;
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream, ToSocketAddrs, UdpSocket};
use std::time::Duration;

use byteorder::{BigEndian, ByteOrder};
use gethostname::gethostname;
#[cfg(all(unix, not(target_os = "redox")))]
use nix::unistd::sethostname;
use num_bigint::Sign;
use num_traits::ToPrimitive;

use crate::function::PyFuncArgs;
use crate::obj::objbytes::PyBytesRef;
use crate::obj::objint::PyIntRef;
use crate::obj::objstr::PyStringRef;
use crate::obj::objtuple::PyTupleRef;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{PyObjectRef, PyRef, PyResult, PyValue, TryFromObject};
#[cfg(unix)]
use crate::stdlib::os::convert_nix_error;
use crate::vm::VirtualMachine;

#[derive(Debug, Copy, Clone)]
enum AddressFamily {
    Unix = 1,
    Inet = 2,
    Inet6 = 3,
}

impl TryFromObject for AddressFamily {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        match i32::try_from_object(vm, obj)? {
            1 => Ok(AddressFamily::Unix),
            2 => Ok(AddressFamily::Inet),
            3 => Ok(AddressFamily::Inet6),
            value => Err(vm.new_os_error(format!("Unknown address family value: {}", value))),
        }
    }
}

#[derive(Debug, Copy, Clone)]
enum SocketKind {
    Stream = 1,
    Dgram = 2,
}

impl TryFromObject for SocketKind {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        match i32::try_from_object(vm, obj)? {
            1 => Ok(SocketKind::Stream),
            2 => Ok(SocketKind::Dgram),
            value => Err(vm.new_os_error(format!("Unknown socket kind value: {}", value))),
        }
    }
}

#[derive(Debug)]
enum Connection {
    TcpListener(TcpListener),
    TcpStream(TcpStream),
    UdpSocket(UdpSocket),
}

impl Connection {
    fn accept(&mut self) -> io::Result<(TcpStream, SocketAddr)> {
        match self {
            Connection::TcpListener(con) => con.accept(),
            _ => Err(io::Error::new(io::ErrorKind::Other, "oh no!")),
        }
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        match self {
            Connection::TcpListener(con) => con.local_addr(),
            Connection::UdpSocket(con) => con.local_addr(),
            Connection::TcpStream(con) => con.local_addr(),
        }
    }

    fn recv_from(&self, buf: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        match self {
            Connection::UdpSocket(con) => con.recv_from(buf),
            _ => Err(io::Error::new(io::ErrorKind::Other, "oh no!")),
        }
    }

    fn send_to<A: ToSocketAddrs>(&self, buf: &[u8], addr: A) -> io::Result<usize> {
        match self {
            Connection::UdpSocket(con) => con.send_to(buf, addr),
            _ => Err(io::Error::new(io::ErrorKind::Other, "oh no!")),
        }
    }

    #[cfg(unix)]
    fn fileno(&self) -> i64 {
        use std::os::unix::io::AsRawFd;
        let raw_fd = match self {
            Connection::TcpListener(con) => con.as_raw_fd(),
            Connection::UdpSocket(con) => con.as_raw_fd(),
            Connection::TcpStream(con) => con.as_raw_fd(),
        };
        i64::from(raw_fd)
    }

    #[cfg(windows)]
    fn fileno(&self) -> i64 {
        use std::os::windows::io::AsRawSocket;
        let raw_fd = match self {
            Connection::TcpListener(con) => con.as_raw_socket(),
            Connection::UdpSocket(con) => con.as_raw_socket(),
            Connection::TcpStream(con) => con.as_raw_socket(),
        };
        raw_fd as i64
    }

    #[cfg(all(not(unix), not(windows)))]
    fn fileno(&self) -> i64 {
        unimplemented!();
    }

    fn setblocking(&mut self, value: bool) -> io::Result<()> {
        match self {
            Connection::TcpListener(con) => con.set_nonblocking(!value),
            Connection::UdpSocket(con) => con.set_nonblocking(!value),
            Connection::TcpStream(con) => con.set_nonblocking(!value),
        }
    }

    fn settimeout(&mut self, duration: Duration) -> io::Result<()> {
        match self {
            // net
            Connection::TcpListener(_con) => Ok(()),
            Connection::UdpSocket(con) => con
                .set_read_timeout(Some(duration))
                .and_then(|_| con.set_write_timeout(Some(duration))),
            Connection::TcpStream(con) => con
                .set_read_timeout(Some(duration))
                .and_then(|_| con.set_write_timeout(Some(duration))),
        }
    }
}

impl Read for Connection {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Connection::TcpStream(con) => con.read(buf),
            Connection::UdpSocket(con) => con.recv(buf),
            _ => Err(io::Error::new(io::ErrorKind::Other, "oh no!")),
        }
    }
}

impl Write for Connection {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Connection::TcpStream(con) => con.write(buf),
            Connection::UdpSocket(con) => con.send(buf),
            _ => Err(io::Error::new(io::ErrorKind::Other, "oh no!")),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Debug)]
pub struct Socket {
    address_family: AddressFamily,
    socket_kind: SocketKind,
    con: RefCell<Option<Connection>>,
    timeout: RefCell<Option<Duration>>,
}

impl PyValue for Socket {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("socket", "socket")
    }
}

impl Socket {
    fn new(address_family: AddressFamily, socket_kind: SocketKind) -> Socket {
        Socket {
            address_family,
            socket_kind,
            con: RefCell::new(None),
            timeout: RefCell::new(None),
        }
    }
}

type SocketRef = PyRef<Socket>;

impl SocketRef {
    fn new(
        cls: PyClassRef,
        family: AddressFamily,
        kind: SocketKind,
        vm: &VirtualMachine,
    ) -> PyResult<SocketRef> {
        Socket::new(family, kind).into_ref_with_type(vm, cls)
    }

    fn enter(self, _vm: &VirtualMachine) -> SocketRef {
        self
    }

    fn exit(self, _args: PyFuncArgs, _vm: &VirtualMachine) {
        self.close(_vm)
    }

    fn connect(self, address: Address, vm: &VirtualMachine) -> PyResult<()> {
        let address_string = address.get_address_string();

        match self.socket_kind {
            SocketKind::Stream => {
                let con = if let Some(duration) = self.timeout.borrow().as_ref() {
                    let sock_addr = match address_string.to_socket_addrs() {
                        Ok(mut sock_addrs) => {
                            if sock_addrs.len() == 0 {
                                let error_type = vm.class("socket", "gaierror");
                                return Err(vm.new_exception(
                                    error_type,
                                    "nodename nor servname provided, or not known".to_string(),
                                ));
                            } else {
                                sock_addrs.next().unwrap()
                            }
                        }
                        Err(e) => {
                            let error_type = vm.class("socket", "gaierror");
                            return Err(vm.new_exception(error_type, e.to_string()));
                        }
                    };
                    TcpStream::connect_timeout(&sock_addr, *duration)
                } else {
                    TcpStream::connect(address_string)
                };
                match con {
                    Ok(stream) => {
                        self.con.borrow_mut().replace(Connection::TcpStream(stream));
                        Ok(())
                    }
                    Err(ref e) if e.kind() == io::ErrorKind::TimedOut => {
                        let socket_timeout = vm.class("socket", "timeout");
                        Err(vm.new_exception(socket_timeout, "Timed out".to_string()))
                    }
                    Err(s) => Err(vm.new_os_error(s.to_string())),
                }
            }
            SocketKind::Dgram => {
                if let Some(Connection::UdpSocket(con)) = self.con.borrow().as_ref() {
                    match con.connect(address_string) {
                        Ok(_) => Ok(()),
                        Err(s) => Err(vm.new_os_error(s.to_string())),
                    }
                } else {
                    Err(vm.new_type_error("".to_string()))
                }
            }
        }
    }

    fn bind(self, address: Address, vm: &VirtualMachine) -> PyResult<()> {
        let address_string = address.get_address_string();

        match self.socket_kind {
            SocketKind::Stream => match TcpListener::bind(address_string) {
                Ok(stream) => {
                    self.con
                        .borrow_mut()
                        .replace(Connection::TcpListener(stream));
                    Ok(())
                }
                Err(s) => Err(vm.new_os_error(s.to_string())),
            },
            SocketKind::Dgram => match UdpSocket::bind(address_string) {
                Ok(dgram) => {
                    self.con.borrow_mut().replace(Connection::UdpSocket(dgram));
                    Ok(())
                }
                Err(s) => Err(vm.new_os_error(s.to_string())),
            },
        }
    }

    fn listen(self, _num: PyIntRef, _vm: &VirtualMachine) {}

    fn accept(self, vm: &VirtualMachine) -> PyResult {
        let ret = match self.con.borrow_mut().as_mut() {
            Some(v) => v.accept(),
            None => return Err(vm.new_type_error("".to_string())),
        };

        let (tcp_stream, addr) = match ret {
            Ok((socket, addr)) => (socket, addr),
            Err(s) => return Err(vm.new_os_error(s.to_string())),
        };

        let socket = Socket {
            address_family: self.address_family,
            socket_kind: self.socket_kind,
            con: RefCell::new(Some(Connection::TcpStream(tcp_stream))),
            timeout: RefCell::new(None),
        }
        .into_ref(vm);

        let addr_tuple = get_addr_tuple(vm, addr)?;

        Ok(vm.ctx.new_tuple(vec![socket.into_object(), addr_tuple]))
    }

    fn recv(self, bufsize: PyIntRef, vm: &VirtualMachine) -> PyResult {
        let mut buffer = vec![0u8; bufsize.as_bigint().to_usize().unwrap()];
        match self.con.borrow_mut().as_mut() {
            Some(v) => match v.read_exact(&mut buffer) {
                Ok(_) => (),
                Err(ref e) if e.kind() == io::ErrorKind::TimedOut => {
                    let socket_timeout = vm.class("socket", "timeout");
                    return Err(vm.new_exception(socket_timeout, "Timed out".to_string()));
                }
                Err(s) => return Err(vm.new_os_error(s.to_string())),
            },
            None => return Err(vm.new_type_error("".to_string())),
        };
        Ok(vm.ctx.new_bytes(buffer))
    }

    fn recvfrom(self, bufsize: PyIntRef, vm: &VirtualMachine) -> PyResult {
        let mut buffer = vec![0u8; bufsize.as_bigint().to_usize().unwrap()];
        let ret = match self.con.borrow().as_ref() {
            Some(v) => v.recv_from(&mut buffer),
            None => return Err(vm.new_type_error("".to_string())),
        };

        let addr = match ret {
            Ok((_size, addr)) => addr,
            Err(s) => return Err(vm.new_os_error(s.to_string())),
        };

        let addr_tuple = get_addr_tuple(vm, addr)?;

        Ok(vm.ctx.new_tuple(vec![vm.ctx.new_bytes(buffer), addr_tuple]))
    }

    fn send(self, bytes: PyBytesRef, vm: &VirtualMachine) -> PyResult<()> {
        match self.con.borrow_mut().as_mut() {
            Some(v) => match v.write(&bytes) {
                Ok(_) => (),
                Err(ref e) if e.kind() == io::ErrorKind::TimedOut => {
                    let socket_timeout = vm.class("socket", "timeout");
                    return Err(vm.new_exception(socket_timeout, "Timed out".to_string()));
                }
                Err(s) => return Err(vm.new_os_error(s.to_string())),
            },
            None => return Err(vm.new_type_error("Socket is not connected".to_string())),
        };
        Ok(())
    }

    fn sendto(self, bytes: PyBytesRef, address: Address, vm: &VirtualMachine) -> PyResult<()> {
        let address_string = address.get_address_string();

        match self.socket_kind {
            SocketKind::Dgram => {
                if let Some(v) = self.con.borrow().as_ref() {
                    return match v.send_to(&bytes, address_string) {
                        Ok(_) => Ok(()),
                        Err(s) => Err(vm.new_os_error(s.to_string())),
                    };
                }
                // Doing implicit bind
                match UdpSocket::bind("0.0.0.0:0") {
                    Ok(dgram) => match dgram.send_to(&bytes, address_string) {
                        Ok(_) => {
                            self.con.borrow_mut().replace(Connection::UdpSocket(dgram));
                            Ok(())
                        }
                        Err(s) => Err(vm.new_os_error(s.to_string())),
                    },
                    Err(s) => Err(vm.new_os_error(s.to_string())),
                }
            }
            _ => Err(vm.new_not_implemented_error("".to_string())),
        }
    }

    fn close(self, _vm: &VirtualMachine) {
        self.con.borrow_mut().take();
    }

    fn fileno(self, vm: &VirtualMachine) -> PyResult {
        let fileno = match self.con.borrow_mut().as_mut() {
            Some(v) => v.fileno(),
            None => return Err(vm.new_type_error("".to_string())),
        };
        Ok(vm.ctx.new_int(fileno))
    }

    fn getsockname(self, vm: &VirtualMachine) -> PyResult {
        let addr = match self.con.borrow().as_ref() {
            Some(v) => v.local_addr(),
            None => return Err(vm.new_type_error("".to_string())),
        };

        match addr {
            Ok(addr) => get_addr_tuple(vm, addr),
            Err(s) => Err(vm.new_os_error(s.to_string())),
        }
    }

    fn gettimeout(self, _vm: &VirtualMachine) -> PyResult<Option<f64>> {
        match self.timeout.borrow().as_ref() {
            Some(duration) => Ok(Some(duration.as_secs() as f64)),
            None => Ok(None),
        }
    }

    fn setblocking(self, block: Option<bool>, vm: &VirtualMachine) -> PyResult<()> {
        match block {
            Some(value) => {
                if value {
                    self.timeout.replace(None);
                } else {
                    self.timeout.borrow_mut().replace(Duration::from_secs(0));
                }
                if let Some(conn) = self.con.borrow_mut().as_mut() {
                    match conn.setblocking(value) {
                        Ok(_) => Ok(()),
                        Err(err) => Err(vm.new_os_error(err.to_string())),
                    }
                } else {
                    Ok(())
                }
            }
            None => {
                // Avoid converting None to bool
                Err(vm.new_type_error("an bool is required".to_string()))
            }
        }
    }

    fn getblocking(self, _vm: &VirtualMachine) -> PyResult<Option<bool>> {
        match self.timeout.borrow().as_ref() {
            Some(duration) => {
                if duration.as_secs() != 0 {
                    Ok(Some(true))
                } else {
                    Ok(Some(false))
                }
            }
            None => Ok(Some(true)),
        }
    }

    fn settimeout(self, timeout: Option<f64>, vm: &VirtualMachine) -> PyResult<()> {
        match timeout {
            Some(timeout) => {
                self.timeout
                    .borrow_mut()
                    .replace(Duration::from_secs(timeout as u64));

                let block = timeout > 0.0;

                if let Some(conn) = self.con.borrow_mut().as_mut() {
                    conn.setblocking(block)
                        .and_then(|_| conn.settimeout(Duration::from_secs(timeout as u64)))
                        .map_err(|err| vm.new_os_error(err.to_string()))
                        .map(|_| ())
                } else {
                    Ok(())
                }
            }
            None => {
                self.timeout.replace(None);
                Ok(())
            }
        }
    }
}

struct Address {
    host: String,
    port: usize,
}

impl Address {
    fn get_address_string(self) -> String {
        format!("{}:{}", self.host, self.port.to_string())
    }
}

impl TryFromObject for Address {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        let tuple = PyTupleRef::try_from_object(vm, obj)?;
        if tuple.elements.len() != 2 {
            Err(vm.new_type_error("Address tuple should have only 2 values".to_string()))
        } else {
            Ok(Address {
                host: PyStringRef::try_from_object(vm, tuple.elements[0].clone())?
                    .as_str()
                    .to_owned(),
                port: PyIntRef::try_from_object(vm, tuple.elements[1].clone())?
                    .as_bigint()
                    .to_usize()
                    .unwrap(),
            })
        }
    }
}

fn get_addr_tuple(vm: &VirtualMachine, addr: SocketAddr) -> PyResult {
    let port = vm.ctx.new_int(addr.port());
    let ip = vm.ctx.new_str(addr.ip().to_string());

    Ok(vm.ctx.new_tuple(vec![ip, port]))
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
    let ip_num = BigEndian::read_u32(packed_ip.get_value());
    Ok(vm.new_str(Ipv4Addr::from(ip_num).to_string()))
}

fn socket_htonl(host: PyIntRef, vm: &VirtualMachine) -> PyResult {
    if host.as_bigint().sign() == Sign::Minus {
        return Err(
            vm.new_overflow_error("can't convert negative value to unsigned int".to_string())
        );
    }

    let host = host.as_bigint().to_u32().unwrap();
    Ok(vm.new_int(host.to_be()))
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;
    let socket_timeout = ctx.new_class("socket.timeout", vm.ctx.exceptions.os_error.clone());
    let socket_gaierror = ctx.new_class("socket.gaierror", vm.ctx.exceptions.os_error.clone());

    let socket = py_class!(ctx, "socket", ctx.object(), {
        (slot new) => SocketRef::new,
        "__enter__" => ctx.new_rustfunc(SocketRef::enter),
        "__exit__" => ctx.new_rustfunc(SocketRef::exit),
        "connect" => ctx.new_rustfunc(SocketRef::connect),
        "recv" => ctx.new_rustfunc(SocketRef::recv),
        "send" => ctx.new_rustfunc(SocketRef::send),
        "bind" => ctx.new_rustfunc(SocketRef::bind),
        "accept" => ctx.new_rustfunc(SocketRef::accept),
        "listen" => ctx.new_rustfunc(SocketRef::listen),
        "close" => ctx.new_rustfunc(SocketRef::close),
        "getsockname" => ctx.new_rustfunc(SocketRef::getsockname),
        "sendto" => ctx.new_rustfunc(SocketRef::sendto),
        "recvfrom" => ctx.new_rustfunc(SocketRef::recvfrom),
        "fileno" => ctx.new_rustfunc(SocketRef::fileno),
        "getblocking" => ctx.new_rustfunc(SocketRef::getblocking),
        "setblocking" => ctx.new_rustfunc(SocketRef::setblocking),
        "gettimeout" => ctx.new_rustfunc(SocketRef::gettimeout),
        "settimeout" => ctx.new_rustfunc(SocketRef::settimeout),
    });

    let module = py_module!(vm, "socket", {
        "error" => ctx.exceptions.os_error.clone(),
        "timeout" => socket_timeout,
        "gaierror" => socket_gaierror,
        "AF_INET" => ctx.new_int(AddressFamily::Inet as i32),
        "SOCK_STREAM" => ctx.new_int(SocketKind::Stream as i32),
        "SOCK_DGRAM" => ctx.new_int(SocketKind::Dgram as i32),
        "socket" => socket,
        "inet_aton" => ctx.new_rustfunc(socket_inet_aton),
        "inet_ntoa" => ctx.new_rustfunc(socket_inet_ntoa),
        "gethostname" => ctx.new_rustfunc(socket_gethostname),
        "htonl" => ctx.new_rustfunc(socket_htonl),
    });

    extend_module_platform_specific(vm, module)
}

#[cfg(not(unix))]
fn extend_module_platform_specific(_vm: &VirtualMachine, module: PyObjectRef) -> PyObjectRef {
    module
}

#[cfg(unix)]
fn extend_module_platform_specific(vm: &VirtualMachine, module: PyObjectRef) -> PyObjectRef {
    let ctx = &vm.ctx;

    #[cfg(not(target_os = "redox"))]
    extend_module!(vm, module, {
        "sethostname" => ctx.new_rustfunc(socket_sethostname),
    });

    module
}
