use std::cell::RefCell;
use std::io;
use std::io::Read;
use std::io::Write;
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs, UdpSocket};
use std::ops::Deref;

use crate::function::PyFuncArgs;
use crate::obj::objbytes;
use crate::obj::objint;
use crate::obj::objsequence::get_elements;
use crate::obj::objstr;
use crate::pyobject::{PyObjectRef, PyRef, PyResult, PyValue, TryFromObject, TypeProtocol};
use crate::vm::VirtualMachine;

use crate::obj::objtype::PyClassRef;
use num_traits::ToPrimitive;

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
        }
    }
}

fn get_socket<'a>(obj: &'a PyObjectRef) -> impl Deref<Target = Socket> + 'a {
    obj.payload::<Socket>().unwrap()
}

type SocketRef = PyRef<Socket>;

fn socket_new(
    cls: PyClassRef,
    family: AddressFamily,
    kind: SocketKind,
    vm: &VirtualMachine,
) -> PyResult<SocketRef> {
    Socket::new(family, kind).into_ref_with_type(vm, cls)
}

fn socket_connect(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, None), (address, Some(vm.ctx.tuple_type()))]
    );

    let address_string = get_address_string(vm, address)?;

    let socket = get_socket(zelf);

    match socket.socket_kind {
        SocketKind::Stream => match TcpStream::connect(address_string) {
            Ok(stream) => {
                socket
                    .con
                    .borrow_mut()
                    .replace(Connection::TcpStream(stream));
                Ok(vm.get_none())
            }
            Err(s) => Err(vm.new_os_error(s.to_string())),
        },
        SocketKind::Dgram => {
            if let Some(Connection::UdpSocket(con)) = socket.con.borrow().as_ref() {
                match con.connect(address_string) {
                    Ok(_) => Ok(vm.get_none()),
                    Err(s) => Err(vm.new_os_error(s.to_string())),
                }
            } else {
                Err(vm.new_type_error("".to_string()))
            }
        }
    }
}

fn socket_bind(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, None), (address, Some(vm.ctx.tuple_type()))]
    );

    let address_string = get_address_string(vm, address)?;

    let socket = get_socket(zelf);

    match socket.socket_kind {
        SocketKind::Stream => match TcpListener::bind(address_string) {
            Ok(stream) => {
                socket
                    .con
                    .borrow_mut()
                    .replace(Connection::TcpListener(stream));
                Ok(vm.get_none())
            }
            Err(s) => Err(vm.new_os_error(s.to_string())),
        },
        SocketKind::Dgram => match UdpSocket::bind(address_string) {
            Ok(dgram) => {
                socket
                    .con
                    .borrow_mut()
                    .replace(Connection::UdpSocket(dgram));
                Ok(vm.get_none())
            }
            Err(s) => Err(vm.new_os_error(s.to_string())),
        },
    }
}

fn get_address_string(vm: &VirtualMachine, address: &PyObjectRef) -> Result<String, PyObjectRef> {
    let args = PyFuncArgs {
        args: get_elements(address).to_vec(),
        kwargs: vec![],
    };
    arg_check!(
        vm,
        args,
        required = [
            (host, Some(vm.ctx.str_type())),
            (port, Some(vm.ctx.int_type()))
        ]
    );

    Ok(format!(
        "{}:{}",
        objstr::get_value(host),
        objint::get_value(port).to_string()
    ))
}

fn socket_listen(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(_zelf, None), (_num, Some(vm.ctx.int_type()))]
    );
    Ok(vm.get_none())
}

fn socket_accept(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, None)]);

    let socket = get_socket(zelf);

    let ret = match socket.con.borrow_mut().as_mut() {
        Some(v) => v.accept(),
        None => return Err(vm.new_type_error("".to_string())),
    };

    let (tcp_stream, addr) = match ret {
        Ok((socket, addr)) => (socket, addr),
        Err(s) => return Err(vm.new_os_error(s.to_string())),
    };

    let socket = Socket {
        address_family: socket.address_family,
        socket_kind: socket.socket_kind,
        con: RefCell::new(Some(Connection::TcpStream(tcp_stream))),
    }
    .into_ref(vm);

    let addr_tuple = get_addr_tuple(vm, addr)?;

    Ok(vm.ctx.new_tuple(vec![socket.into_object(), addr_tuple]))
}

fn socket_recv(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, None), (bufsize, Some(vm.ctx.int_type()))]
    );
    let socket = get_socket(zelf);

    let mut buffer = vec![0u8; objint::get_value(bufsize).to_usize().unwrap()];
    match socket.con.borrow_mut().as_mut() {
        Some(v) => match v.read_exact(&mut buffer) {
            Ok(_) => (),
            Err(s) => return Err(vm.new_os_error(s.to_string())),
        },
        None => return Err(vm.new_type_error("".to_string())),
    };
    Ok(vm.ctx.new_bytes(buffer))
}

fn socket_recvfrom(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, None), (bufsize, Some(vm.ctx.int_type()))]
    );

    let socket = get_socket(zelf);

    let mut buffer = vec![0u8; objint::get_value(bufsize).to_usize().unwrap()];
    let ret = match socket.con.borrow().as_ref() {
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

fn socket_send(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, None), (bytes, Some(vm.ctx.bytes_type()))]
    );
    let socket = get_socket(zelf);

    match socket.con.borrow_mut().as_mut() {
        Some(v) => match v.write(&objbytes::get_value(&bytes)) {
            Ok(_) => (),
            Err(s) => return Err(vm.new_os_error(s.to_string())),
        },
        None => return Err(vm.new_type_error("".to_string())),
    };
    Ok(vm.get_none())
}

fn socket_sendto(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [
            (zelf, None),
            (bytes, Some(vm.ctx.bytes_type())),
            (address, Some(vm.ctx.tuple_type()))
        ]
    );
    let address_string = get_address_string(vm, address)?;

    let socket = get_socket(zelf);

    match socket.socket_kind {
        SocketKind::Dgram => {
            if let Some(v) = socket.con.borrow().as_ref() {
                return match v.send_to(&objbytes::get_value(&bytes), address_string) {
                    Ok(_) => Ok(vm.get_none()),
                    Err(s) => Err(vm.new_os_error(s.to_string())),
                };
            }
            // Doing implicit bind
            match UdpSocket::bind("0.0.0.0:0") {
                Ok(dgram) => match dgram.send_to(&objbytes::get_value(&bytes), address_string) {
                    Ok(_) => {
                        socket
                            .con
                            .borrow_mut()
                            .replace(Connection::UdpSocket(dgram));
                        Ok(vm.get_none())
                    }
                    Err(s) => Err(vm.new_os_error(s.to_string())),
                },
                Err(s) => Err(vm.new_os_error(s.to_string())),
            }
        }
        _ => Err(vm.new_not_implemented_error("".to_string())),
    }
}

fn socket_close(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, None)]);

    let socket = get_socket(zelf);
    socket.con.borrow_mut().take();
    Ok(vm.get_none())
}

fn socket_getsockname(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, None)]);
    let socket = get_socket(zelf);

    let addr = match socket.con.borrow().as_ref() {
        Some(v) => v.local_addr(),
        None => return Err(vm.new_type_error("".to_string())),
    };

    match addr {
        Ok(addr) => get_addr_tuple(vm, addr),
        Err(s) => Err(vm.new_os_error(s.to_string())),
    }
}

fn get_addr_tuple(vm: &VirtualMachine, addr: SocketAddr) -> PyResult {
    let port = vm.ctx.new_int(addr.port());
    let ip = vm.ctx.new_str(addr.ip().to_string());

    Ok(vm.ctx.new_tuple(vec![ip, port]))
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    let socket = py_class!(ctx, "socket", ctx.object(), {
         "__new__" => ctx.new_rustfunc(socket_new),
         "connect" => ctx.new_rustfunc(socket_connect),
         "recv" => ctx.new_rustfunc(socket_recv),
         "send" => ctx.new_rustfunc(socket_send),
         "bind" => ctx.new_rustfunc(socket_bind),
         "accept" => ctx.new_rustfunc(socket_accept),
         "listen" => ctx.new_rustfunc(socket_listen),
         "close" => ctx.new_rustfunc(socket_close),
         "getsockname" => ctx.new_rustfunc(socket_getsockname),
         "sendto" => ctx.new_rustfunc(socket_sendto),
         "recvfrom" => ctx.new_rustfunc(socket_recvfrom),
    });

    py_module!(vm, "socket", {
        "AF_INET" => ctx.new_int(AddressFamily::Inet as i32),
        "SOCK_STREAM" => ctx.new_int(SocketKind::Stream as i32),
         "SOCK_DGRAM" => ctx.new_int(SocketKind::Dgram as i32),
         "socket" => socket,
    })
}
