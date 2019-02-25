use std::cell::RefCell;
use std::io;
use std::io::Read;
use std::io::Write;
use std::net::{SocketAddr, TcpListener, TcpStream, UdpSocket};

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
    AfUnix = 1,
    AfInet = 2,
    AfInet6 = 3,
}

impl AddressFamily {
    fn from_i32(value: i32) -> AddressFamily {
        match value {
            1 => AddressFamily::AfUnix,
            2 => AddressFamily::AfInet,
            3 => AddressFamily::AfInet6,
            _ => panic!("Unknown value: {}", value),
        }
    }
}

#[derive(Copy, Clone)]
enum SocketKind {
    SockStream = 1,
    SockDgram = 2,
}

impl SocketKind {
    fn from_i32(value: i32) -> SocketKind {
        match value {
            1 => SocketKind::SockStream,
            2 => SocketKind::SockDgram,
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
}

impl Read for Connection {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Connection::TcpStream(con) => con.read(buf),
            _ => Err(io::Error::new(io::ErrorKind::Other, "oh no!")),
        }
    }
}

impl Write for Connection {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Connection::TcpStream(con) => con.write(buf),
            _ => Err(io::Error::new(io::ErrorKind::Other, "oh no!")),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub struct Socket {
    address_family: AddressFamily,
    sk: SocketKind,
    con: Option<Connection>,
}

impl Socket {
    fn new(address_family: AddressFamily, sk: SocketKind) -> Socket {
        Socket {
            address_family,
            sk: sk,
            con: None,
        }
    }
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
        PyObjectPayload::Socket { socket },
        cls.clone(),
    ))
}

fn socket_connect(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, None), (address, Some(vm.ctx.tuple_type()))]
    );

    let elements = get_elements(address);
    let host = objstr::get_value(&elements[0]);
    let port = objint::get_value(&elements[1]);

    let address_string = format!("{}:{}", host, port.to_string());

    match zelf.payload {
        PyObjectPayload::Socket { ref socket } => {
            if let Ok(stream) = TcpStream::connect(address_string) {
                socket.borrow_mut().con = Some(Connection::TcpStream(stream));
                Ok(vm.get_none())
            } else {
                // TODO: Socket error
                Err(vm.new_type_error("socket failed".to_string()))
            }
        }
        _ => Err(vm.new_type_error("".to_string())),
    }
}

fn socket_bind(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, None), (address, Some(vm.ctx.tuple_type()))]
    );

    let elements = get_elements(address);
    let host = objstr::get_value(&elements[0]);
    let port = objint::get_value(&elements[1]);

    let address_string = format!("{}:{}", host, port.to_string());

    match zelf.payload {
        PyObjectPayload::Socket { ref socket } => {
            if let Ok(stream) = TcpListener::bind(address_string) {
                socket.borrow_mut().con = Some(Connection::TcpListener(stream));
                Ok(vm.get_none())
            } else {
                // TODO: Socket error
                Err(vm.new_type_error("socket failed".to_string()))
            }
        }
        _ => Err(vm.new_type_error("".to_string())),
    }
}

fn socket_listen(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    Ok(vm.get_none())
}

fn socket_accept(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, None)]);

    match zelf.payload {
        PyObjectPayload::Socket { ref socket } => {
            let ret = match socket.borrow_mut().con {
                Some(ref mut v) => v.accept(),
                None => return Err(vm.new_type_error("".to_string())),
            };

            let tcp_stream = match ret {
                Ok((socket, _addr)) => socket,
                _ => return Err(vm.new_type_error("".to_string())),
            };

            let socket = RefCell::new(Socket {
                address_family: socket.borrow().address_family.clone(),
                sk: socket.borrow().sk.clone(),
                con: Some(Connection::TcpStream(tcp_stream)),
            });

            let sock_obj = PyObject::new(PyObjectPayload::Socket { socket }, zelf.typ());

            let elements = RefCell::new(vec![sock_obj, vm.get_none()]);

            Ok(PyObject::new(
                PyObjectPayload::Sequence { elements },
                vm.ctx.tuple_type(),
            ))
        }
        _ => Err(vm.new_type_error("".to_string())),
    }
}

fn socket_recv(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, None), (bufsize, Some(vm.ctx.int_type()))]
    );
    match zelf.payload {
        PyObjectPayload::Socket { ref socket } => {
            let mut buffer = vec![0u8; objint::get_value(bufsize).to_usize().unwrap()];
            match socket.borrow_mut().con {
                Some(ref mut v) => v.read_exact(&mut buffer).unwrap(),
                None => return Err(vm.new_type_error("".to_string())),
            };
            Ok(vm.ctx.new_bytes(buffer))
        }
        _ => Err(vm.new_type_error("".to_string())),
    }
}

fn socket_send(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, None), (bytes, Some(vm.ctx.bytes_type()))]
    );
    match zelf.payload {
        PyObjectPayload::Socket { ref socket } => {
            match socket.borrow_mut().con {
                Some(ref mut v) => v.write(&objbytes::get_value(&bytes)).unwrap(),
                None => return Err(vm.new_type_error("".to_string())),
            };
            Ok(vm.get_none())
        }
        _ => Err(vm.new_type_error("".to_string())),
    }
}

fn socket_close(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, None)]);
    match zelf.payload {
        PyObjectPayload::Socket { ref socket } => {
            let mut socket = socket.borrow_mut();
            match socket.address_family {
                AddressFamily::AfInet => match socket.sk {
                    SocketKind::SockStream => {
                        socket.con = None;
                        Ok(vm.get_none())
                    }
                    _ => Err(vm.new_type_error("".to_string())),
                },
                _ => Err(vm.new_type_error("".to_string())),
            }
        }
        _ => Err(vm.new_type_error("".to_string())),
    }
}

pub fn mk_module(ctx: &PyContext) -> PyObjectRef {
    let py_mod = ctx.new_module(&"socket".to_string(), ctx.new_scope(None));

    ctx.set_attr(
        &py_mod,
        "AF_INET",
        ctx.new_int(AddressFamily::AfInet as i32),
    );

    ctx.set_attr(
        &py_mod,
        "SOCK_STREAM",
        ctx.new_int(SocketKind::SockStream as i32),
    );

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
        socket
    };
    ctx.set_attr(&py_mod, "socket", socket.clone());

    py_mod
}
