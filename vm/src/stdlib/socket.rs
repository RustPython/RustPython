use crate::obj::objbytes;
use crate::obj::objint;
use crate::obj::objstr;

use crate::pyobject::{
    PyContext, PyFuncArgs, PyObject, PyObjectPayload, PyObjectRef, PyResult, TypeProtocol,
};
use crate::vm::VirtualMachine;

use num_traits::ToPrimitive;
use std::fmt;
use std::io::Read;
use std::io::Write;
use std::net::{TcpListener, TcpStream, UdpSocket};

#[derive(Debug)]
enum AddressFamily {
    AF_UNIX = 1,
    AF_INET = 2,
    AF_INET6 = 3,
}

impl AddressFamily {
    fn from_i32(value: i32) -> AddressFamily {
        match value {
            1 => AddressFamily::AF_UNIX,
            2 => AddressFamily::AF_INET,
            3 => AddressFamily::AF_INET6,
            _ => panic!("Unknown value: {}", value),
        }
    }
}

#[derive(Debug)]
enum SocketKind {
    SOCK_STREAM = 1,
    SOCK_DGRAM = 2,
}

impl SocketKind {
    fn from_i32(value: i32) -> SocketKind {
        match value {
            1 => SocketKind::SOCK_STREAM,
            2 => SocketKind::SOCK_DGRAM,
            _ => panic!("Unknown value: {}", value),
        }
    }
}

impl fmt::Display for AddressFamily {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl fmt::Display for SocketKind {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

pub struct Socket {
    af: AddressFamily,
    sk: SocketKind,
    tcp_listener: Option<TcpListener>,
    tcp_stream: Option<TcpStream>,
    udp_socket: Option<UdpSocket>,
}

impl Socket {
    fn new(af: AddressFamily, sk: SocketKind) -> Socket {
        Socket {
            af: af,
            sk: sk,
            tcp_listener: None,
            tcp_stream: None,
            udp_socket: None,
        }
    }

    fn get_tcp_stream(&self) -> Option<&TcpStream> {
        match &self.tcp_stream {
            Some(v) => Some(v),
            None => None,
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

    let family = AddressFamily::from_i32(objint::get_value(family_int).to_i32().unwrap());
    let kind = SocketKind::from_i32(objint::get_value(kind_int).to_i32().unwrap());

    let socket = Socket::new(family, kind);

    Ok(PyObject::new(
        PyObjectPayload::Socket { socket },
        cls.clone(),
    ))
}

fn socket_connect(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, None), (address, Some(vm.ctx.str_type()))]
    );

    let mut mut_obj = zelf.borrow_mut();

    match mut_obj.payload {
        PyObjectPayload::Socket { ref mut socket } => {
            if let Ok(stream) = TcpStream::connect(objstr::get_value(&address)) {
                socket.tcp_stream = Some(stream);
                Ok(vm.get_none())
            } else {
                // TODO: Socket error
                Err(vm.new_type_error("socket failed".to_string()))
            }
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
    let mut mut_obj = zelf.borrow_mut();

    match mut_obj.payload {
        PyObjectPayload::Socket { ref mut socket } => match socket.af {
            AddressFamily::AF_INET => match socket.sk {
                SocketKind::SOCK_STREAM => {
                    let mut buffer = Vec::new();
                    socket
                        .get_tcp_stream()
                        .unwrap()
                        .read_to_end(&mut buffer)
                        .unwrap();
                    Ok(PyObject::new(
                        PyObjectPayload::Bytes { value: buffer },
                        vm.ctx.bytes_type(),
                    ))
                }
                _ => Err(vm.new_type_error("".to_string())),
            },
            _ => Err(vm.new_type_error("".to_string())),
        },
        _ => Err(vm.new_type_error("".to_string())),
    }
}

fn socket_send(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, None), (bytes, Some(vm.ctx.bytes_type()))]
    );
    let mut mut_obj = zelf.borrow_mut();

    match mut_obj.payload {
        PyObjectPayload::Socket { ref mut socket } => match socket.af {
            AddressFamily::AF_INET => match socket.sk {
                SocketKind::SOCK_STREAM => {
                    socket
                        .get_tcp_stream()
                        .unwrap()
                        .write(&objbytes::get_value(&bytes))
                        .unwrap();
                    Ok(vm.get_none())
                }
                _ => Err(vm.new_type_error("".to_string())),
            },
            _ => Err(vm.new_type_error("".to_string())),
        },
        _ => Err(vm.new_type_error("".to_string())),
    }
}

fn socket_close(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, None)]);
    let mut mut_obj = zelf.borrow_mut();

    match mut_obj.payload {
        PyObjectPayload::Socket { ref mut socket } => match socket.af {
            AddressFamily::AF_INET => match socket.sk {
                SocketKind::SOCK_STREAM => {
                    socket.tcp_stream = None;
                    Ok(vm.get_none())
                }
                _ => Err(vm.new_type_error("".to_string())),
            },
            _ => Err(vm.new_type_error("".to_string())),
        },
        _ => Err(vm.new_type_error("".to_string())),
    }
}

pub fn mk_module(ctx: &PyContext) -> PyObjectRef {
    let py_mod = ctx.new_module(&"socket".to_string(), ctx.new_scope(None));

    ctx.set_attr(
        &py_mod,
        &AddressFamily::AF_INET.to_string(),
        ctx.new_int(AddressFamily::AF_INET as i32),
    );

    ctx.set_attr(
        &py_mod,
        &SocketKind::SOCK_STREAM.to_string(),
        ctx.new_int(SocketKind::SOCK_STREAM as i32),
    );

    let socket = {
        let socket = ctx.new_class("socket", ctx.object());
        ctx.set_attr(&socket, "__new__", ctx.new_rustfunc(socket_new));
        ctx.set_attr(&socket, "connect", ctx.new_rustfunc(socket_connect));
        ctx.set_attr(&socket, "recv", ctx.new_rustfunc(socket_recv));
        ctx.set_attr(&socket, "send", ctx.new_rustfunc(socket_send));
        ctx.set_attr(&socket, "close", ctx.new_rustfunc(socket_close));
        socket
    };
    ctx.set_attr(&py_mod, "socket", socket.clone());

    py_mod
}
