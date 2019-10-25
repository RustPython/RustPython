use std::cell::Cell;
use std::io;
use std::net::{Ipv4Addr, Shutdown, SocketAddr, ToSocketAddrs};
use std::time::Duration;

use byteorder::{BigEndian, ByteOrder};
use gethostname::gethostname;
#[cfg(all(unix, not(target_os = "redox")))]
use nix::unistd::sethostname;
use socket2::{Domain, Socket, Type as SocketType};

use super::os::convert_io_error;
#[cfg(unix)]
use super::os::convert_nix_error;
use super::time_module::duration_to_f64;
use crate::function::OptionalArg;
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

#[pyclass]
#[derive(Debug)]
pub struct PySocket {
    timeout: Cell<Option<Duration>>,
    sock: Socket,
}

impl PyValue for PySocket {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("_socket", "socket")
    }
}

pub type PySocketRef = PyRef<PySocket>;

#[pyimpl]
impl PySocket {
    #[pyslot(new)]
    fn new(
        cls: PyClassRef,
        domain: OptionalArg<i32>,
        socket_type: OptionalArg<i32>,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<Self>> {
        let domain = domain.unwrap_or(libc::AF_INET);
        let socket_type = socket_type.unwrap_or(libc::SOCK_STREAM);
        let domain = match domain {
            libc::AF_INET => Domain::ipv4(),
            libc::AF_INET6 => Domain::ipv6(),
            #[cfg(unix)]
            libc::AF_UNIX => Domain::unix(),
            _ => return Err(vm.new_os_error(format!("Unknown address family value: {}", domain))),
        };
        let socket_type = match socket_type {
            libc::SOCK_STREAM => SocketType::stream(),
            libc::SOCK_DGRAM => SocketType::dgram(),
            _ => return Err(vm.new_os_error(format!("Unknown socket kind value: {}", socket_type))),
        };
        let sock =
            Socket::new(domain, socket_type, None).map_err(|err| convert_io_error(vm, err))?;
        PySocket {
            sock,
            timeout: Cell::default(),
        }
        .into_ref_with_type(vm, cls)
    }

    #[pymethod]
    fn connect(&self, address: Address, vm: &VirtualMachine) -> PyResult<()> {
        let sock_addr = get_addr(vm, address)?;
        let res = if let Some(duration) = self.timeout.get() {
            self.sock.connect_timeout(&sock_addr, duration)
        } else {
            self.sock.connect(&sock_addr)
        };
        res.map_err(|err| convert_io_error(vm, err))
    }

    #[pymethod]
    fn bind(&self, address: Address, vm: &VirtualMachine) -> PyResult<()> {
        let sock_addr = get_addr(vm, address)?;
        self.sock
            .bind(&sock_addr)
            .map_err(|err| convert_io_error(vm, err))
    }

    #[pymethod]
    fn listen(&self, backlog: OptionalArg<i32>, vm: &VirtualMachine) -> PyResult<()> {
        let backlog = backlog.unwrap_or(128);
        let backlog = if backlog < 0 { 0 } else { backlog };
        self.sock
            .listen(backlog)
            .map_err(|err| convert_io_error(vm, err))
    }

    #[pymethod]
    fn accept(&self, vm: &VirtualMachine) -> PyResult {
        let (sock, addr) = self
            .sock
            .accept()
            .map_err(|err| convert_io_error(vm, err))?;

        let socket = PySocket {
            sock,
            timeout: self.timeout.clone(),
        }
        .into_ref(vm);

        let addr_tuple = get_addr_tuple(vm, addr);

        Ok(vm.ctx.new_tuple(vec![socket.into_object(), addr_tuple]))
    }

    #[pymethod]
    fn recv(&self, bufsize: usize, vm: &VirtualMachine) -> PyResult {
        let mut buffer = vec![0u8; bufsize];
        match self.sock.recv(&mut buffer) {
            Ok(_) => Ok(vm.ctx.new_bytes(buffer)),
            Err(ref e) if e.kind() == io::ErrorKind::TimedOut => {
                let socket_timeout = vm.class("_socket", "timeout");
                Err(vm.new_exception(socket_timeout, "Timed out".to_string()))
            }
            Err(err) => Err(convert_io_error(vm, err)),
        }
    }

    #[pymethod]
    fn recvfrom(&self, bufsize: usize, vm: &VirtualMachine) -> PyResult {
        let mut buffer = vec![0u8; bufsize];
        let addr = match self.sock.recv_from(&mut buffer) {
            Ok((_, addr)) => addr,
            Err(ref e) if e.kind() == io::ErrorKind::TimedOut => {
                let socket_timeout = vm.class("_socket", "timeout");
                return Err(vm.new_exception(socket_timeout, "Timed out".to_string()));
            }
            Err(err) => return Err(convert_io_error(vm, err)),
        };

        let addr_tuple = get_addr_tuple(vm, addr);

        Ok(vm.ctx.new_tuple(vec![vm.ctx.new_bytes(buffer), addr_tuple]))
    }

    #[pymethod]
    fn send(&self, bytes: PyBytesLike, vm: &VirtualMachine) -> PyResult<usize> {
        match self.sock.send(bytes.to_cow().as_ref()) {
            Ok(i) => Ok(i),
            Err(ref e) if e.kind() == io::ErrorKind::TimedOut => {
                let socket_timeout = vm.class("_socket", "timeout");
                Err(vm.new_exception(socket_timeout, "Timed out".to_string()))
            }
            Err(err) => Err(convert_io_error(vm, err)),
        }
    }

    #[pymethod]
    fn sendto(&self, bytes: PyBytesLike, address: Address, vm: &VirtualMachine) -> PyResult<()> {
        let addr = get_addr(vm, address)?;
        self.sock
            .send_to(bytes.to_cow().as_ref(), &addr)
            .map_err(|err| convert_io_error(vm, err))?;
        Ok(())
    }

    #[pymethod]
    fn close(&self, vm: &VirtualMachine) -> PyResult<()> {
        let fd = self.clone().fileno(vm);
        #[cfg(unix)]
        let ret = unsafe { libc::close(fd) };
        #[cfg(windows)]
        let ret = {
            extern "system" {
                fn closesocket(s: std::os::windows::raw::SOCKET) -> c_int;
            };
            unsafe { closesocket(fd) }
        };
        if ret != 0 {
            Err(convert_io_error(vm, io::Error::last_os_error()))
        } else {
            Ok(())
        }
    }

    #[pymethod]
    fn fileno(&self, _vm: &VirtualMachine) -> RawSocket {
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            self.sock.as_raw_fd()
        }
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawSocket;
            self.sock.as_raw_socket()
        }
    }

    #[pymethod]
    fn getsockname(&self, vm: &VirtualMachine) -> PyResult {
        let addr = self
            .sock
            .local_addr()
            .map_err(|err| convert_io_error(vm, err))?;

        Ok(get_addr_tuple(vm, addr))
    }

    #[pymethod]
    fn gettimeout(&self, _vm: &VirtualMachine) -> Option<f64> {
        self.timeout.get().map(duration_to_f64)
    }

    #[pymethod]
    fn setblocking(&self, block: bool, vm: &VirtualMachine) -> PyResult<()> {
        self.sock
            .set_nonblocking(!block)
            .map_err(|err| convert_io_error(vm, err))?;
        if block {
            self.timeout.set(None);
        } else {
            self.timeout.set(Some(Duration::from_secs(0)));
        }
        Ok(())
    }

    #[pymethod]
    fn getblocking(&self, _vm: &VirtualMachine) -> bool {
        self.timeout.get().map_or(false, |d| d.as_secs() != 0)
    }

    #[pymethod]
    fn settimeout(&self, timeout: f64, vm: &VirtualMachine) -> PyResult<()> {
        let secs: u64 = timeout.trunc() as u64;
        let nanos: u32 = (timeout.fract() * 1e9) as u32;
        let duration = Duration::new(secs, nanos);

        self.timeout.set(Some(duration));

        self.sock
            .set_nonblocking(duration.as_secs() == 0)
            .map_err(|err| convert_io_error(vm, err))?;

        Ok(())
    }

    #[pymethod]
    fn shutdown(&self, how: i32, vm: &VirtualMachine) -> PyResult<()> {
        let how = match how {
            libc::SHUT_RD => Shutdown::Read,
            libc::SHUT_WR => Shutdown::Write,
            libc::SHUT_RDWR => Shutdown::Both,
            _ => {
                return Err(
                    vm.new_value_error("`how` must be SHUT_RD, SHUT_WR, or SHUT_RDWR".to_string())
                )
            }
        };
        self.sock
            .shutdown(how)
            .map_err(|err| convert_io_error(vm, err))
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

fn get_addr_tuple<A: Into<socket2::SockAddr>>(vm: &VirtualMachine, addr: A) -> PyObjectRef {
    let addr = addr.into();
    let (port, ip) = if let Some(addr) = addr.as_inet() {
        (addr.port(), addr.ip().to_string())
    } else if let Some(addr) = addr.as_inet6() {
        (addr.port(), addr.ip().to_string())
    } else {
        (0, String::new())
    };
    let port = vm.ctx.new_int(port);
    let ip = vm.ctx.new_str(ip);

    vm.ctx.new_tuple(vec![ip, port])
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

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;
    let socket_timeout = ctx.new_class("socket.timeout", vm.ctx.exceptions.os_error.clone());
    let socket_gaierror = ctx.new_class("socket.gaierror", vm.ctx.exceptions.os_error.clone());

    let module = py_module!(vm, "_socket", {
        "error" => ctx.exceptions.os_error.clone(),
        "timeout" => socket_timeout,
        "gaierror" => socket_gaierror,
        "AF_INET" => ctx.new_int(libc::AF_INET),
        "AF_INET6" => ctx.new_int(libc::AF_INET6),
        "SOCK_STREAM" => ctx.new_int(libc::SOCK_STREAM),
        "SOCK_DGRAM" => ctx.new_int(libc::SOCK_DGRAM),
        "SHUT_RD" => ctx.new_int(libc::SHUT_RD),
        "SHUT_WR" => ctx.new_int(libc::SHUT_WR),
        "SHUT_RDWR" => ctx.new_int(libc::SHUT_RDWR),
        "socket" => PySocket::make_class(ctx),
        "inet_aton" => ctx.new_rustfunc(socket_inet_aton),
        "inet_ntoa" => ctx.new_rustfunc(socket_inet_ntoa),
        "gethostname" => ctx.new_rustfunc(socket_gethostname),
        "htonl" => ctx.new_rustfunc(socket_htonl),
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
