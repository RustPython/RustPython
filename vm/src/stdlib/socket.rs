use std::cell::RefCell;
use std::io;
use std::io::Read;
use std::io::Write;
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs, UdpSocket};
use std::ops::DerefMut;

use crate::obj::objbytes;
use crate::obj::objint;
use crate::obj::objsequence::get_elements;
use crate::obj::objstr;
use crate::pyobject::{
    PyContext, PyFuncArgs, PyObject, PyObjectPayload, PyObjectRef, PyResult, TypeProtocol,
};
use crate::vm::VirtualMachine;

use num_traits::ToPrimitive;

#[derive(Copy, Clone)]
enum AddressFamily {
    Unix = 1,
    Inet = 2,
    Inet6 = 3,
}

impl AddressFamily {
    fn from_i32(value: i32) -> AddressFamily {
        match value {
            1 => AddressFamily::Unix,
            2 => AddressFamily::Inet,
            3 => AddressFamily::Inet6,
            _ => panic!("Unknown value: {}", value),
        }
    }
}

#[derive(Copy, Clone)]
enum SocketKind {
    Stream = 1,
    Dgram = 2,
}

impl SocketKind {
    fn from_i32(value: i32) -> SocketKind {
        match value {
            1 => SocketKind::Stream,
            2 => SocketKind::Dgram,
            _ => panic!("Unknown value: {}", value),
        }
    }
}

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

pub struct Socket {
    address_family: AddressFamily,
    socket_kind: SocketKind,
    con: Option<Connection>,
}

impl Socket {
    fn new(address_family: AddressFamily, socket_kind: SocketKind) -> Socket {
        Socket {
            address_family,
            socket_kind,
            con: None,
        }
    }
}

fn get_socket<'a>(obj: &'a PyObjectRef) -> impl DerefMut<Target = Socket> + 'a {
    if let PyObjectPayload::AnyRustValue { ref value } = obj.payload {
        if let Some(socket) = value.downcast_ref::<RefCell<Socket>>() {
            return socket.borrow_mut();
        }
    }
    panic!("Inner error getting socket {:?}", obj);
}

fn socket_new(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [
            (cls, None),
            (family_int, Some(vm.ctx.int_type())),
            (kind_int, Some(vm.ctx.int_type()))
        ]
    );

    let address_family = AddressFamily::from_i32(objint::get_value(family_int).to_i32().unwrap());
    let kind = SocketKind::from_i32(objint::get_value(kind_int).to_i32().unwrap());

    let socket = RefCell::new(Socket::new(address_family, kind));

    Ok(PyObject::new(
        PyObjectPayload::AnyRustValue {
            value: Box::new(socket),
        },
        cls.clone(),
    ))
}

fn socket_connect(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, None), (address, Some(vm.ctx.tuple_type()))]
    );

    let address_string = get_address_string(vm, address)?;

    let mut socket = get_socket(zelf);

    match socket.socket_kind {
        SocketKind::Stream => {
            if let Ok(stream) = TcpStream::connect(address_string) {
                socket.con = Some(Connection::TcpStream(stream));
                Ok(vm.get_none())
            } else {
                // TODO: Socket error
                Err(vm.new_type_error("socket failed".to_string()))
            }
        }
        SocketKind::Dgram => {
            if let Some(Connection::UdpSocket(con)) = &socket.con {
                match con.connect(address_string) {
                    Ok(_) => Ok(vm.get_none()),
                    // TODO: Socket error
                    Err(_) => Err(vm.new_type_error("socket failed".to_string())),
                }
            } else {
                Err(vm.new_type_error("".to_string()))
            }
        }
    }
}

fn socket_bind(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, None), (address, Some(vm.ctx.tuple_type()))]
    );

    let address_string = get_address_string(vm, address)?;

    let mut socket = get_socket(zelf);

    match socket.socket_kind {
        SocketKind::Stream => {
            if let Ok(stream) = TcpListener::bind(address_string) {
                socket.con = Some(Connection::TcpListener(stream));
                Ok(vm.get_none())
            } else {
                // TODO: Socket error
                Err(vm.new_type_error("socket failed".to_string()))
            }
        }
        SocketKind::Dgram => {
            if let Ok(dgram) = UdpSocket::bind(address_string) {
                socket.con = Some(Connection::UdpSocket(dgram));
                Ok(vm.get_none())
            } else {
                // TODO: Socket error
                Err(vm.new_type_error("socket failed".to_string()))
            }
        }
    }
}

fn get_address_string(
    vm: &mut VirtualMachine,
    address: &PyObjectRef,
) -> Result<String, PyObjectRef> {
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

fn socket_listen(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(_zelf, None), (_num, Some(vm.ctx.int_type()))]
    );
    Ok(vm.get_none())
}

fn socket_accept(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, None)]);

    let mut socket = get_socket(zelf);

    let ret = match socket.con {
        Some(ref mut v) => v.accept(),
        None => return Err(vm.new_type_error("".to_string())),
    };

    let (tcp_stream, addr) = match ret {
        Ok((socket, addr)) => (socket, addr),
        _ => return Err(vm.new_type_error("".to_string())),
    };

    let socket = RefCell::new(Socket {
        address_family: socket.address_family,
        socket_kind: socket.socket_kind,
        con: Some(Connection::TcpStream(tcp_stream)),
    });

    let sock_obj = PyObject::new(
        PyObjectPayload::AnyRustValue {
            value: Box::new(socket),
        },
        zelf.typ(),
    );

    let addr_tuple = get_addr_tuple(vm, addr)?;

    Ok(vm.ctx.new_tuple(vec![sock_obj, addr_tuple]))
}

fn socket_recv(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, None), (bufsize, Some(vm.ctx.int_type()))]
    );
    let mut socket = get_socket(zelf);

    let mut buffer = vec![0u8; objint::get_value(bufsize).to_usize().unwrap()];
    match socket.con {
        Some(ref mut v) => v.read_exact(&mut buffer).unwrap(),
        None => return Err(vm.new_type_error("".to_string())),
    };
    Ok(vm.ctx.new_bytes(buffer))
}

fn socket_recvfrom(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, None), (bufsize, Some(vm.ctx.int_type()))]
    );

    let mut socket = get_socket(zelf);

    let mut buffer = vec![0u8; objint::get_value(bufsize).to_usize().unwrap()];
    let ret = match socket.con {
        Some(ref mut v) => v.recv_from(&mut buffer),
        None => return Err(vm.new_type_error("".to_string())),
    };

    let addr = match ret {
        Ok((_size, addr)) => addr,
        _ => return Err(vm.new_type_error("".to_string())),
    };

    let addr_tuple = get_addr_tuple(vm, addr)?;

    Ok(vm.ctx.new_tuple(vec![vm.ctx.new_bytes(buffer), addr_tuple]))
}

fn socket_send(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, None), (bytes, Some(vm.ctx.bytes_type()))]
    );
    let mut socket = get_socket(zelf);

    match socket.con {
        Some(ref mut v) => v.write(&objbytes::get_value(&bytes)).unwrap(),
        None => return Err(vm.new_type_error("".to_string())),
    };
    Ok(vm.get_none())
}

fn socket_sendto(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
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

    let mut socket = get_socket(zelf);

    match socket.socket_kind {
        SocketKind::Dgram => {
            match socket.con {
                Some(ref mut v) => {
                    if let Ok(_) = v.send_to(&objbytes::get_value(&bytes), address_string) {
                        Ok(vm.get_none())
                    } else {
                        Err(vm.new_type_error("socket failed".to_string()))
                    }
                }
                None => {
                    // Doing implicit bind
                    if let Ok(dgram) = UdpSocket::bind("0.0.0.0:0") {
                        if let Ok(_) = dgram.send_to(&objbytes::get_value(&bytes), address_string) {
                            socket.con = Some(Connection::UdpSocket(dgram));
                            return Ok(vm.get_none());
                        }
                    }
                    Err(vm.new_type_error("socket failed".to_string()))
                }
            }
        }
        _ => Err(vm.new_not_implemented_error("".to_string())),
    }
}

fn socket_close(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, None)]);

    let mut socket = get_socket(zelf);
    socket.con = None;
    Ok(vm.get_none())
}

fn socket_getsockname(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, None)]);
    let mut socket = get_socket(zelf);

    let addr = match socket.con {
        Some(ref mut v) => v.local_addr(),
        None => return Err(vm.new_type_error("".to_string())),
    };

    match addr {
        Ok(addr) => get_addr_tuple(vm, addr),
        _ => Err(vm.new_type_error("".to_string())),
    }
}

fn get_addr_tuple(vm: &mut VirtualMachine, addr: SocketAddr) -> PyResult {
    let port = vm.ctx.new_int(addr.port());
    let ip = vm.ctx.new_str(addr.ip().to_string());

    Ok(vm.ctx.new_tuple(vec![ip, port]))
}

pub fn mk_module(ctx: &PyContext) -> PyObjectRef {
    let py_mod = ctx.new_module(&"socket".to_string(), ctx.new_scope(None));

    ctx.set_attr(&py_mod, "AF_INET", ctx.new_int(AddressFamily::Inet as i32));

    ctx.set_attr(
        &py_mod,
        "SOCK_STREAM",
        ctx.new_int(SocketKind::Stream as i32),
    );

    ctx.set_attr(&py_mod, "SOCK_DGRAM", ctx.new_int(SocketKind::Dgram as i32));

    let socket = {
        let socket = ctx.new_class("socket", ctx.object());
        ctx.set_attr(&socket, "__new__", ctx.new_rustfunc(socket_new));
        ctx.set_attr(&socket, "connect", ctx.new_rustfunc(socket_connect));
        ctx.set_attr(&socket, "recv", ctx.new_rustfunc(socket_recv));
        ctx.set_attr(&socket, "send", ctx.new_rustfunc(socket_send));
        ctx.set_attr(&socket, "bind", ctx.new_rustfunc(socket_bind));
        ctx.set_attr(&socket, "accept", ctx.new_rustfunc(socket_accept));
        ctx.set_attr(&socket, "listen", ctx.new_rustfunc(socket_listen));
        ctx.set_attr(&socket, "close", ctx.new_rustfunc(socket_close));
        ctx.set_attr(&socket, "getsockname", ctx.new_rustfunc(socket_getsockname));
        ctx.set_attr(&socket, "sendto", ctx.new_rustfunc(socket_sendto));
        ctx.set_attr(&socket, "recvfrom", ctx.new_rustfunc(socket_recvfrom));
        socket
    };
    ctx.set_attr(&py_mod, "socket", socket.clone());

    py_mod
}
