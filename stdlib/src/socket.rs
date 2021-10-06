use crate::common::lock::{PyMappedRwLockReadGuard, PyRwLock, PyRwLockReadGuard};
use crate::vm::{
    builtins::{PyBaseExceptionRef, PyStrRef, PyTupleRef, PyTypeRef},
    extend_module,
    function::{
        ArgBytesLike, ArgMemoryBuffer, FuncArgs, IntoPyException, IntoPyObject, OptionalArg,
        OptionalOption,
    },
    named_function, py_module,
    utils::{Either, ToCString},
    PyClassImpl, PyObjectRef, PyResult, PyValue, TryFromBorrowedObject, TryFromObject,
    TypeProtocol, VirtualMachine,
};
use crossbeam_utils::atomic::AtomicCell;
use gethostname::gethostname;
#[cfg(all(unix, not(target_os = "redox")))]
use nix::unistd::sethostname;
use num_traits::ToPrimitive;
use socket2::{Domain, Protocol, Socket, Type as SocketType};
use std::convert::TryFrom;
use std::mem::MaybeUninit;
use std::net::{self, Ipv4Addr, Ipv6Addr, Shutdown, SocketAddr, ToSocketAddrs};
use std::time::{Duration, Instant};
use std::{
    ffi,
    io::{self, Read, Write},
};

#[cfg(unix)]
type RawSocket = std::os::unix::io::RawFd;
#[cfg(windows)]
type RawSocket = std::os::windows::raw::SOCKET;

#[cfg(unix)]
macro_rules! errcode {
    ($e:ident) => {
        c::$e
    };
}
#[cfg(windows)]
macro_rules! errcode {
    ($e:ident) => {
        paste::paste!(c::[<WSA $e>])
    };
}

#[cfg(unix)]
use libc as c;
#[cfg(windows)]
mod c {
    pub use winapi::shared::ifdef::IF_MAX_STRING_SIZE as IF_NAMESIZE;
    pub use winapi::shared::netioapi::{if_indextoname, if_nametoindex};
    pub use winapi::shared::ws2def::*;
    pub use winapi::um::winsock2::{
        SD_BOTH as SHUT_RDWR, SD_RECEIVE as SHUT_RD, SD_SEND as SHUT_WR, SOCK_DGRAM, SOCK_RAW,
        SOCK_RDM, SOCK_SEQPACKET, SOCK_STREAM, SOL_SOCKET, SO_BROADCAST, SO_ERROR, SO_LINGER,
        SO_OOBINLINE, SO_REUSEADDR, SO_TYPE, *,
    };
}
#[cfg(windows)]
use winapi::shared::netioapi;

fn get_raw_sock(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<RawSocket> {
    #[cfg(unix)]
    type CastFrom = libc::c_long;
    #[cfg(windows)]
    type CastFrom = libc::c_longlong;

    // should really just be to_index() but test_socket tests the error messages explicitly
    if obj.isinstance(&vm.ctx.types.float_type) {
        return Err(vm.new_type_error("integer argument expected, got float".to_owned()));
    }
    let int = vm
        .to_index_opt(obj)
        .unwrap_or_else(|| Err(vm.new_type_error("an integer is required".to_owned())))?;
    int.try_to_primitive::<CastFrom>(vm)
        .map(|sock| sock as RawSocket)
}

#[cfg(unix)]
mod nullable_socket {
    use super::*;
    use std::os::unix::io::AsRawFd;

    #[derive(Debug)]
    #[repr(transparent)]
    pub struct NullableSocket(Option<socket2::Socket>);
    impl From<socket2::Socket> for NullableSocket {
        fn from(sock: socket2::Socket) -> Self {
            NullableSocket(Some(sock))
        }
    }
    impl NullableSocket {
        pub fn invalid() -> Self {
            Self(None)
        }
        pub fn get(&self) -> Option<&socket2::Socket> {
            self.0.as_ref()
        }
        pub fn fd(&self) -> RawSocket {
            self.get().map_or(INVALID_SOCKET, |sock| sock.as_raw_fd())
        }
        pub fn insert(&mut self, sock: socket2::Socket) -> &mut socket2::Socket {
            self.0.insert(sock)
        }
    }
}
#[cfg(windows)]
mod nullable_socket {
    use super::*;
    use std::os::windows::io::{AsRawSocket, FromRawSocket};

    // TODO: may change if windows changes its TcpStream repr
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct NullableSocket(socket2::Socket);
    impl From<socket2::Socket> for NullableSocket {
        fn from(sock: socket2::Socket) -> Self {
            NullableSocket(sock)
        }
    }
    impl NullableSocket {
        pub fn invalid() -> Self {
            // TODO: may become UB in the future; maybe see rust-lang/rust#74699
            Self(unsafe { socket2::Socket::from_raw_socket(INVALID_SOCKET) })
        }
        pub fn get(&self) -> Option<&socket2::Socket> {
            (self.0.as_raw_socket() != INVALID_SOCKET).then(|| &self.0)
        }
        pub fn fd(&self) -> RawSocket {
            self.0.as_raw_socket()
        }
        pub fn insert(&mut self, sock: socket2::Socket) -> &mut socket2::Socket {
            self.0 = sock;
            &mut self.0
        }
    }
}
use nullable_socket::NullableSocket;
impl Default for NullableSocket {
    fn default() -> Self {
        Self::invalid()
    }
}

#[pyclass(module = "socket", name = "socket")]
#[derive(Debug, PyValue)]
pub struct PySocket {
    kind: AtomicCell<i32>,
    family: AtomicCell<i32>,
    proto: AtomicCell<i32>,
    pub(crate) timeout: AtomicCell<f64>,
    sock: PyRwLock<NullableSocket>,
}

impl Default for PySocket {
    fn default() -> Self {
        PySocket {
            kind: AtomicCell::default(),
            family: AtomicCell::default(),
            proto: AtomicCell::default(),
            timeout: AtomicCell::new(-1.0),
            sock: PyRwLock::new(NullableSocket::invalid()),
        }
    }
}

#[cfg(windows)]
const CLOSED_ERR: i32 = c::WSAENOTSOCK;
#[cfg(unix)]
const CLOSED_ERR: i32 = c::EBADF;

impl Read for &PySocket {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        (&mut &*self.sock_io()?).read(buf)
    }
}
impl Write for &PySocket {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        (&mut &*self.sock_io()?).write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        (&mut &*self.sock_io()?).flush()
    }
}

impl PySocket {
    pub fn sock_opt(&self) -> Option<PyMappedRwLockReadGuard<'_, Socket>> {
        PyRwLockReadGuard::try_map(self.sock.read(), |sock| sock.get()).ok()
    }

    fn sock_io(&self) -> io::Result<PyMappedRwLockReadGuard<'_, Socket>> {
        self.sock_opt()
            .ok_or_else(|| io::Error::from_raw_os_error(CLOSED_ERR))
    }

    pub fn sock(&self, vm: &VirtualMachine) -> PyResult<PyMappedRwLockReadGuard<'_, Socket>> {
        self.sock_io().map_err(|e| e.into_pyexception(vm))
    }

    fn init_inner(
        &self,
        family: i32,
        socket_kind: i32,
        proto: i32,
        sock: Socket,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        self.family.store(family);
        self.kind.store(socket_kind);
        self.proto.store(proto);
        let mut s = self.sock.write();
        let sock = s.insert(sock);
        let timeout = DEFAULT_TIMEOUT.load();
        self.timeout.store(timeout);
        if timeout >= 0.0 {
            sock.set_nonblocking(true)
                .map_err(|e| e.into_pyexception(vm))?;
        }
        Ok(())
    }

    #[inline]
    fn sock_op<F, R>(&self, vm: &VirtualMachine, select: SelectKind, f: F) -> PyResult<R>
    where
        F: FnMut() -> io::Result<R>,
    {
        self.sock_op_err(vm, select, f)
            .map_err(|e| e.into_pyexception(vm))
    }

    /// returns Err(blocking)
    pub fn get_timeout(&self) -> Result<Duration, bool> {
        let timeout = self.timeout.load();
        if timeout > 0.0 {
            Ok(Duration::from_secs_f64(timeout))
        } else {
            Err(timeout != 0.0)
        }
    }

    fn sock_op_err<F, R>(
        &self,
        vm: &VirtualMachine,
        select: SelectKind,
        f: F,
    ) -> Result<R, IoOrPyException>
    where
        F: FnMut() -> io::Result<R>,
    {
        self.sock_op_timeout_err(vm, select, self.get_timeout().ok(), f)
    }

    fn sock_op_timeout_err<F, R>(
        &self,
        vm: &VirtualMachine,
        select: SelectKind,
        timeout: Option<Duration>,
        mut f: F,
    ) -> Result<R, IoOrPyException>
    where
        F: FnMut() -> io::Result<R>,
    {
        let deadline = timeout.map(Deadline::new);

        loop {
            if deadline.is_some() || matches!(select, SelectKind::Connect) {
                let interval = deadline.as_ref().map(|d| d.time_until()).transpose()?;
                let res = sock_select(&*self.sock(vm)?, select, interval);
                match res {
                    Ok(true) => return Err(IoOrPyException::Timeout),
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {
                        vm.check_signals()?;
                        continue;
                    }
                    Err(e) => return Err(e.into()),
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
            return Err(err.into());
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
            #[cfg(unix)]
            c::AF_UNIX => {
                use std::os::unix::ffi::OsStrExt;
                let buf = crate::vm::function::ArgStrOrBytesLike::try_from_object(vm, addr)?;
                let path = &*buf.borrow_bytes();
                if cfg!(any(target_os = "linux", target_os = "android")) && path.first() == Some(&0)
                {
                    use libc::{sa_family_t, socklen_t};
                    use {socket2::SockAddr, std::ptr};
                    unsafe {
                        // based on SockAddr::unix
                        // TODO: upstream or fix socklen check for SockAddr::unix()?
                        SockAddr::init(|storage, len| {
                            // Safety: `SockAddr::init` zeros the address, which is a valid
                            // representation.
                            let storage: &mut libc::sockaddr_un = &mut *storage.cast();
                            let len: &mut socklen_t = &mut *len;

                            let bytes = path;
                            if bytes.len() > storage.sun_path.len() {
                                return Err(io::Error::new(
                                    io::ErrorKind::InvalidInput,
                                    "path must be shorter than SUN_LEN",
                                ));
                            }

                            storage.sun_family = libc::AF_UNIX as sa_family_t;
                            // Safety: `bytes` and `addr.sun_path` are not overlapping and
                            // both point to valid memory.
                            // `SockAddr::init` zeroes the memory, so the path is already
                            // null terminated.
                            ptr::copy_nonoverlapping(
                                bytes.as_ptr(),
                                storage.sun_path.as_mut_ptr() as *mut u8,
                                bytes.len(),
                            );

                            let base = storage as *const _ as usize;
                            let path = &storage.sun_path as *const _ as usize;
                            let sun_path_offset = path - base;
                            let length = sun_path_offset + bytes.len();
                            *len = length as socklen_t;

                            Ok(())
                        })
                    }
                    .map(|(_, addr)| addr)
                } else {
                    socket2::SockAddr::unix(ffi::OsStr::from_bytes(path))
                }
                .map_err(|_| vm.new_os_error("AF_UNIX path too long".to_owned()))
            }
            c::AF_INET => {
                let tuple: PyTupleRef = addr.downcast().map_err(|obj| {
                    vm.new_type_error(format!(
                        "{}(): AF_INET address must be tuple, not {}",
                        caller,
                        obj.class().name()
                    ))
                })?;
                let tuple = tuple.as_slice();
                if tuple.len() != 2 {
                    return Err(
                        vm.new_type_error("AF_INET address must be a pair (host, post)".to_owned())
                    );
                }
                let addr = Address::from_tuple(tuple, vm)?;
                let mut addr4 = get_addr(vm, addr.host, c::AF_INET)?;
                match &mut addr4 {
                    SocketAddr::V4(addr4) => {
                        addr4.set_port(addr.port);
                    }
                    SocketAddr::V6(_) => unreachable!(),
                }
                Ok(addr4.into())
            }
            c::AF_INET6 => {
                let tuple: PyTupleRef = addr.downcast().map_err(|obj| {
                    vm.new_type_error(format!(
                        "{}(): AF_INET6 address must be tuple, not {}",
                        caller,
                        obj.class().name()
                    ))
                })?;
                let tuple = tuple.as_slice();
                match tuple.len() {
                    2 | 3 | 4 => {}
                    _ => {
                        return Err(vm.new_type_error(
                            "AF_INET6 address must be a tuple (host, port[, flowinfo[, scopeid]])"
                                .to_owned(),
                        ))
                    }
                }
                let (addr, flowinfo, scopeid) = Address::from_tuple_ipv6(tuple, vm)?;
                let mut addr6 = get_addr(vm, addr.host, c::AF_INET6)?;
                match &mut addr6 {
                    SocketAddr::V6(addr6) => {
                        addr6.set_port(addr.port);
                        addr6.set_flowinfo(flowinfo);
                        addr6.set_scope_id(scopeid);
                    }
                    SocketAddr::V4(_) => unreachable!(),
                }
                Ok(addr6.into())
            }
            _ => Err(vm.new_os_error(format!("{}(): bad family", caller))),
        }
    }

    fn connect_inner(
        &self,
        address: PyObjectRef,
        caller: &str,
        vm: &VirtualMachine,
    ) -> Result<(), IoOrPyException> {
        let sock_addr = self.extract_address(address, caller, vm)?;

        let err = match self.sock(vm)?.connect(&sock_addr) {
            Ok(()) => return Ok(()),
            Err(e) => e,
        };

        let wait_connect = if err.kind() == io::ErrorKind::Interrupted {
            vm.check_signals()?;
            self.timeout.load() != 0.0
        } else {
            #[cfg(unix)]
            use c::EINPROGRESS;
            #[cfg(windows)]
            use c::WSAEWOULDBLOCK as EINPROGRESS;

            self.timeout.load() > 0.0 && err.raw_os_error() == Some(EINPROGRESS)
        };

        if wait_connect {
            // basically, connect() is async, and it registers an "error" on the socket when it's
            // done connecting. SelectKind::Connect fills the errorfds fd_set, so if we wake up
            // from poll and the error is EISCONN then we know that the connect is done
            self.sock_op_err(vm, SelectKind::Connect, || {
                let sock = self.sock_io()?;
                let err = sock.take_error()?;
                match err {
                    Some(e) if e.raw_os_error() == Some(libc::EISCONN) => Ok(()),
                    Some(e) => Err(e),
                    // TODO: is this accurate?
                    None => Ok(()),
                }
            })
        } else {
            Err(err.into())
        }
    }
}

#[pyimpl(flags(BASETYPE))]
impl PySocket {
    #[pyslot]
    fn slot_new(cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        Self::default().into_pyresult_with_type(vm, cls)
    }

    #[pymethod(magic)]
    fn init(
        &self,
        family: OptionalArg<i32>,
        socket_kind: OptionalArg<i32>,
        proto: OptionalArg<i32>,
        fileno: OptionalOption<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let mut family = family.unwrap_or(-1);
        let mut socket_kind = socket_kind.unwrap_or(-1);
        let mut proto = proto.unwrap_or(-1);
        let fileno = fileno
            .flatten()
            .map(|obj| get_raw_sock(obj, vm))
            .transpose()?;
        let sock;
        if let Some(fileno) = fileno {
            sock = sock_from_raw(fileno, vm)?;
            match sock.local_addr() {
                Ok(addr) if family == -1 => family = addr.family() as i32,
                Err(e)
                    if family == -1
                        || matches!(
                            e.raw_os_error(),
                            Some(errcode!(ENOTSOCK)) | Some(errcode!(EBADF))
                        ) =>
                {
                    std::mem::forget(sock);
                    return Err(e.into_pyexception(vm));
                }
                _ => {}
            }
            if socket_kind == -1 {
                // TODO: when socket2 cuts a new release, type will be available on all os
                // socket_kind = sock.r#type().map_err(|e| e.into_pyexception(vm))?.into();
                let res = unsafe {
                    c::getsockopt(
                        sock_fileno(&sock) as _,
                        c::SOL_SOCKET,
                        c::SO_TYPE,
                        &mut socket_kind as *mut libc::c_int as *mut _,
                        &mut (std::mem::size_of::<i32>() as _),
                    )
                };
                if res < 0 {
                    return Err(crate::vm::stdlib::os::errno_err(vm));
                }
            }
            cfg_if::cfg_if! {
                if #[cfg(any(
                    target_os = "android",
                    target_os = "freebsd",
                    target_os = "fuchsia",
                    target_os = "linux",
                ))] {
                    if proto == -1 {
                        proto = sock.protocol().map_err(|e| e.into_pyexception(vm))?.map_or(0, Into::into);
                    }
                } else {
                    proto = 0;
                }
            }
        } else {
            if family == -1 {
                family = c::AF_INET as i32
            }
            if socket_kind == -1 {
                socket_kind = c::SOCK_STREAM
            }
            if proto == -1 {
                proto = 0
            }
            sock = Socket::new(
                Domain::from(family),
                SocketType::from(socket_kind),
                Some(Protocol::from(proto)),
            )
            .map_err(|err| err.into_pyexception(vm))?;
        };
        self.init_inner(family, socket_kind, proto, sock, vm)
    }

    #[pymethod]
    fn connect(&self, address: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.connect_inner(address, "connect", vm)
            .map_err(|e| e.into_pyexception(vm))
    }

    #[pymethod]
    fn connect_ex(&self, address: PyObjectRef, vm: &VirtualMachine) -> PyResult<i32> {
        match self.connect_inner(address, "connect_ex", vm) {
            Ok(()) => Ok(0),
            Err(err) => err.errno(),
        }
    }

    #[pymethod]
    fn bind(&self, address: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let sock_addr = self.extract_address(address, "bind", vm)?;
        self.sock(vm)?
            .bind(&sock_addr)
            .map_err(|err| err.into_pyexception(vm))
    }

    #[pymethod]
    fn listen(&self, backlog: OptionalArg<i32>, vm: &VirtualMachine) -> PyResult<()> {
        let backlog = backlog.unwrap_or(128);
        let backlog = if backlog < 0 { 0 } else { backlog };
        self.sock(vm)?
            .listen(backlog)
            .map_err(|err| err.into_pyexception(vm))
    }

    #[pymethod]
    fn _accept(&self, vm: &VirtualMachine) -> PyResult<(RawSocket, PyObjectRef)> {
        let (sock, addr) = self.sock_op(vm, SelectKind::Read, || self.sock_io()?.accept())?;
        let fd = into_sock_fileno(sock);
        Ok((fd, get_addr_tuple(&addr, vm)))
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
        let sock = self.sock(vm)?;
        let n = self.sock_op(vm, SelectKind::Read, || {
            sock.recv_with_flags(spare_capacity_mut(&mut buffer), flags)
        })?;
        unsafe { buffer.set_len(n) };
        Ok(buffer)
    }

    #[pymethod]
    fn recv_into(
        &self,
        buf: ArgMemoryBuffer,
        flags: OptionalArg<i32>,
        vm: &VirtualMachine,
    ) -> PyResult<usize> {
        let flags = flags.unwrap_or(0);
        let sock = self.sock(vm)?;
        let mut buf = buf.borrow_buf_mut();
        let buf = &mut *buf;
        self.sock_op(vm, SelectKind::Read, || {
            sock.recv_with_flags(slice_as_uninit(buf), flags)
        })
    }

    #[pymethod]
    fn recvfrom(
        &self,
        bufsize: isize,
        flags: OptionalArg<i32>,
        vm: &VirtualMachine,
    ) -> PyResult<(Vec<u8>, PyObjectRef)> {
        let flags = flags.unwrap_or(0);
        let bufsize = bufsize
            .to_usize()
            .ok_or_else(|| vm.new_value_error("negative buffersize in recvfrom".to_owned()))?;
        let mut buffer = Vec::with_capacity(bufsize);
        let (n, addr) = self.sock_op(vm, SelectKind::Read, || {
            self.sock_io()?
                .recv_from_with_flags(spare_capacity_mut(&mut buffer), flags)
        })?;
        unsafe { buffer.set_len(n) };
        Ok((buffer, get_addr_tuple(&addr, vm)))
    }

    #[pymethod]
    fn recvfrom_into(
        &self,
        buf: ArgMemoryBuffer,
        nbytes: OptionalArg<isize>,
        flags: OptionalArg<i32>,
        vm: &VirtualMachine,
    ) -> PyResult<(usize, PyObjectRef)> {
        let mut buf = buf.borrow_buf_mut();
        let buf = &mut *buf;
        let buf = match nbytes {
            OptionalArg::Present(i) => {
                let i = i.to_usize().ok_or_else(|| {
                    vm.new_value_error("negative buffersize in recvfrom_into".to_owned())
                })?;
                buf.get_mut(..i).ok_or_else(|| {
                    vm.new_value_error("nbytes is greater than the length of the buffer".to_owned())
                })?
            }
            OptionalArg::Missing => buf,
        };
        let flags = flags.unwrap_or(0);
        let sock = self.sock(vm)?;
        let (n, addr) = self.sock_op(vm, SelectKind::Read, || {
            sock.recv_from_with_flags(slice_as_uninit(buf), flags)
        })?;
        Ok((n, get_addr_tuple(&addr, vm)))
    }

    #[pymethod]
    fn send(
        &self,
        bytes: ArgBytesLike,
        flags: OptionalArg<i32>,
        vm: &VirtualMachine,
    ) -> PyResult<usize> {
        let flags = flags.unwrap_or(0);
        let buf = bytes.borrow_buf();
        let buf = &*buf;
        self.sock_op(vm, SelectKind::Write, || {
            self.sock_io()?.send_with_flags(buf, flags)
        })
    }

    #[pymethod]
    fn sendall(
        &self,
        bytes: ArgBytesLike,
        flags: OptionalArg<i32>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let flags = flags.unwrap_or(0);

        let timeout = self.get_timeout().ok();

        let deadline = timeout.map(Deadline::new);

        let buf = bytes.borrow_buf();
        let buf = &*buf;
        let mut buf_offset = 0;
        // now we have like 3 layers of interrupt loop :)
        while buf_offset < buf.len() {
            let interval = deadline
                .as_ref()
                .map(|d| d.time_until().map_err(|e| e.into_pyexception(vm)))
                .transpose()?;
            self.sock_op_timeout_err(vm, SelectKind::Write, interval, || {
                let subbuf = &buf[buf_offset..];
                buf_offset += self.sock_io()?.send_with_flags(subbuf, flags)?;
                Ok(())
            })
            .map_err(|e| e.into_pyexception(vm))?;
            vm.check_signals()?;
        }
        Ok(())
    }

    #[pymethod]
    fn sendto(
        &self,
        bytes: ArgBytesLike,
        arg2: PyObjectRef,
        arg3: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<usize> {
        // signature is bytes[, flags], address
        let (flags, address) = match arg3 {
            OptionalArg::Present(arg3) => {
                // should just be i32::try_from_obj but tests check for error message
                let int = vm.to_index_opt(arg2).unwrap_or_else(|| {
                    Err(vm.new_type_error("an integer is required".to_owned()))
                })?;
                let flags = int.try_to_primitive::<i32>(vm)?;
                (flags, arg3)
            }
            OptionalArg::Missing => (0, arg2),
        };
        let addr = self.extract_address(address, "sendto", vm)?;
        let buf = bytes.borrow_buf();
        let buf = &*buf;
        self.sock_op(vm, SelectKind::Write, || {
            self.sock_io()?.send_to_with_flags(buf, &addr, flags)
        })
    }

    #[pymethod]
    fn close(&self, vm: &VirtualMachine) -> PyResult<()> {
        let sock = self.detach();
        if sock != INVALID_SOCKET {
            close_inner(sock, vm)?;
        }
        Ok(())
    }
    #[pymethod]
    #[inline]
    fn detach(&self) -> RawSocket {
        let sock = std::mem::replace(&mut *self.sock.write(), NullableSocket::invalid());
        std::mem::ManuallyDrop::new(sock).fd()
    }

    #[pymethod]
    fn fileno(&self) -> RawSocket {
        self.sock.read().fd()
    }

    #[pymethod]
    fn getsockname(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let addr = self
            .sock(vm)?
            .local_addr()
            .map_err(|err| err.into_pyexception(vm))?;

        Ok(get_addr_tuple(&addr, vm))
    }
    #[pymethod]
    fn getpeername(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let addr = self
            .sock(vm)?
            .peer_addr()
            .map_err(|err| err.into_pyexception(vm))?;

        Ok(get_addr_tuple(&addr, vm))
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
        self.sock(vm)?
            .set_nonblocking(!block)
            .map_err(|err| err.into_pyexception(vm))
    }

    #[pymethod]
    fn getblocking(&self) -> bool {
        self.timeout.load() != 0.0
    }

    #[pymethod]
    fn settimeout(&self, timeout: Option<Duration>, vm: &VirtualMachine) -> PyResult<()> {
        self.timeout
            .store(timeout.map_or(-1.0, |d| d.as_secs_f64()));
        // even if timeout is > 0 the socket needs to be nonblocking in order for us to select() on
        // it
        self.sock(vm)?
            .set_nonblocking(timeout.is_some())
            .map_err(|err| err.into_pyexception(vm))
    }

    #[pymethod]
    fn getsockopt(
        &self,
        level: i32,
        name: i32,
        buflen: OptionalArg<i32>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let fd = self.sock.read().fd() as _;
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
                Err(crate::vm::stdlib::os::errno_err(vm))
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
                Err(crate::vm::stdlib::os::errno_err(vm))
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
        value: Option<Either<ArgBytesLike, i32>>,
        optlen: OptionalArg<u32>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let fd = self.sock.read().fd() as _;
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
            Err(crate::vm::stdlib::os::errno_err(vm))
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
        self.sock(vm)?
            .shutdown(how)
            .map_err(|err| err.into_pyexception(vm))
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

    #[pymethod(magic)]
    fn repr(&self) -> String {
        format!(
            "<socket object, fd={}, family={}, type={}, proto={}>",
            // cast because INVALID_SOCKET is unsigned, so would show usize::MAX instead of -1
            self.sock.read().fd() as i64,
            self.family.load(),
            self.kind.load(),
            self.proto.load(),
        )
    }
}

struct Address {
    host: PyStrRef,
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
        if tuple.as_slice().len() != 2 {
            Err(vm.new_type_error("Address tuple should have only 2 values".to_owned()))
        } else {
            Self::from_tuple(tuple.as_slice(), vm)
        }
    }
}

impl Address {
    fn from_tuple(tuple: &[PyObjectRef], vm: &VirtualMachine) -> PyResult<Self> {
        let host = PyStrRef::try_from_object(vm, tuple[0].clone())?;
        let port = i32::try_from_borrowed_object(vm, &tuple[1])?;
        let port = port
            .to_u16()
            .ok_or_else(|| vm.new_overflow_error("port must be 0-65535.".to_owned()))?;
        Ok(Address { host, port })
    }
    fn from_tuple_ipv6(tuple: &[PyObjectRef], vm: &VirtualMachine) -> PyResult<(Self, u32, u32)> {
        let addr = Address::from_tuple(tuple, vm)?;
        let flowinfo = tuple
            .get(2)
            .map(|obj| u32::try_from_borrowed_object(vm, obj))
            .transpose()?
            .unwrap_or(0);
        let scopeid = tuple
            .get(3)
            .map(|obj| u32::try_from_borrowed_object(vm, obj))
            .transpose()?
            .unwrap_or(0);
        if flowinfo > 0xfffff {
            return Err(vm.new_overflow_error("flowinfo must be 0-1048575.".to_owned()));
        }
        Ok((addr, flowinfo, scopeid))
    }
}

fn get_ip_addr_tuple(addr: &SocketAddr, vm: &VirtualMachine) -> PyObjectRef {
    match addr {
        SocketAddr::V4(addr) => (addr.ip().to_string(), addr.port()).into_pyobject(vm),
        SocketAddr::V6(addr) => (
            addr.ip().to_string(),
            addr.port(),
            addr.flowinfo(),
            addr.scope_id(),
        )
            .into_pyobject(vm),
    }
}

fn get_addr_tuple(addr: &socket2::SockAddr, vm: &VirtualMachine) -> PyObjectRef {
    if let Some(addr) = addr.as_socket() {
        return get_ip_addr_tuple(&addr, vm);
    }
    match addr.family() as i32 {
        #[cfg(unix)]
        libc::AF_UNIX => {
            let addr_len = addr.len() as usize;
            let unix_addr = unsafe { &*(addr.as_ptr() as *const libc::sockaddr_un) };
            let path_u8 = unsafe { &*(&unix_addr.sun_path[..] as *const [_] as *const [u8]) };
            let sun_path_offset =
                &unix_addr.sun_path as *const _ as usize - unix_addr as *const _ as usize;
            if cfg!(any(target_os = "linux", target_os = "android"))
                && addr_len > sun_path_offset
                && unix_addr.sun_path[0] == 0
            {
                let abstractaddrlen = addr_len - sun_path_offset;
                let abstractpath = &path_u8[..abstractaddrlen];
                vm.ctx.new_bytes(abstractpath.to_vec())
            } else {
                let len = memchr::memchr(b'\0', path_u8).unwrap_or_else(|| path_u8.len());
                let path = &path_u8[..len];
                vm.ctx
                    .new_utf8_str(String::from_utf8_lossy(path).into_owned())
            }
        }
        // TODO: support more address families
        _ => (String::new(), 0).into_pyobject(vm),
    }
}

fn _socket_gethostname(vm: &VirtualMachine) -> PyResult {
    gethostname()
        .into_string()
        .map(|hostname| vm.ctx.new_utf8_str(hostname))
        .map_err(|err| vm.new_os_error(err.into_string().unwrap()))
}

#[cfg(all(unix, not(target_os = "redox")))]
fn _socket_sethostname(hostname: PyStrRef, vm: &VirtualMachine) -> PyResult<()> {
    sethostname(hostname.as_str()).map_err(|err| err.into_pyexception(vm))
}

fn _socket_inet_aton(ip_string: PyStrRef, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
    ip_string
        .as_str()
        .parse::<Ipv4Addr>()
        .map(|ip_addr| Vec::<u8>::from(ip_addr.octets()))
        .map_err(|_| vm.new_os_error("illegal IP address string passed to inet_aton".to_owned()))
}

fn _socket_inet_ntoa(packed_ip: ArgBytesLike, vm: &VirtualMachine) -> PyResult {
    let packed_ip = packed_ip.borrow_buf();
    let packed_ip = <&[u8; 4]>::try_from(&*packed_ip)
        .map_err(|_| vm.new_os_error("packed IP wrong length for inet_ntoa".to_owned()))?;
    Ok(vm.ctx.new_utf8_str(Ipv4Addr::from(*packed_ip).to_string()))
}

fn cstr_opt_as_ptr(x: &OptionalArg<ffi::CString>) -> *const libc::c_char {
    x.as_ref().map_or_else(std::ptr::null, |s| s.as_ptr())
}

fn _socket_getservbyname(
    servicename: PyStrRef,
    protocolname: OptionalArg<PyStrRef>,
    vm: &VirtualMachine,
) -> PyResult {
    let cstr_name = servicename.to_cstring(vm)?;
    let cstr_proto = protocolname
        .as_ref()
        .map(|s| s.to_cstring(vm))
        .transpose()?;
    let cstr_proto = cstr_opt_as_ptr(&cstr_proto);
    let serv = unsafe { c::getservbyname(cstr_name.as_ptr(), cstr_proto) };
    if serv.is_null() {
        return Err(vm.new_os_error("service/proto not found".to_owned()));
    }
    let port = unsafe { (*serv).s_port };
    Ok(vm.ctx.new_int(u16::from_be(port as u16)))
}

fn _socket_getservbyport(
    port: i32,
    protocolname: OptionalArg<PyStrRef>,
    vm: &VirtualMachine,
) -> PyResult<String> {
    let port = port
        .to_u16()
        .ok_or_else(|| vm.new_overflow_error("getservbyport: port must be 0-65535.".to_owned()))?;
    let cstr_proto = protocolname
        .as_ref()
        .map(|s| s.to_cstring(vm))
        .transpose()?;
    let cstr_proto = cstr_opt_as_ptr(&cstr_proto);
    let serv = unsafe { c::getservbyport(port.to_be() as _, cstr_proto) };
    if serv.is_null() {
        return Err(vm.new_os_error("port/proto not found".to_owned()));
    }
    let s = unsafe { ffi::CStr::from_ptr((*serv).s_name) };
    Ok(s.to_string_lossy().into_owned())
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

enum IoOrPyException {
    Timeout,
    Py(PyBaseExceptionRef),
    Io(io::Error),
}
impl From<PyBaseExceptionRef> for IoOrPyException {
    fn from(exc: PyBaseExceptionRef) -> Self {
        Self::Py(exc)
    }
}
impl From<io::Error> for IoOrPyException {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}
impl IoOrPyException {
    fn errno(self) -> PyResult<i32> {
        match self {
            Self::Timeout => Ok(errcode!(EWOULDBLOCK)),
            Self::Io(err) => {
                // TODO: just unwrap()?
                Ok(err.raw_os_error().unwrap_or(1))
            }
            Self::Py(exc) => Err(exc),
        }
    }
}
impl IntoPyException for IoOrPyException {
    fn into_pyexception(self, vm: &VirtualMachine) -> PyBaseExceptionRef {
        match self {
            Self::Timeout => timeout_error(vm),
            Self::Py(exc) => exc,
            Self::Io(err) => err.into_pyexception(vm),
        }
    }
}

#[derive(Copy, Clone)]
pub(super) enum SelectKind {
    Read,
    Write,
    Connect,
}

/// returns true if timed out
pub(super) fn sock_select(
    sock: &Socket,
    kind: SelectKind,
    interval: Option<Duration>,
) -> io::Result<bool> {
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
        use crate::select;

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

    #[pyarg(positional, default = "c::AF_UNSPEC")]
    family: i32,
    #[pyarg(positional, default = "0")]
    ty: i32,
    #[pyarg(positional, default = "0")]
    proto: i32,
    #[pyarg(positional, default = "0")]
    flags: i32,
}

fn _socket_getaddrinfo(opts: GAIOptions, vm: &VirtualMachine) -> PyResult {
    let hints = dns_lookup::AddrInfoHints {
        socktype: opts.ty,
        protocol: opts.proto,
        address: opts.family,
        flags: opts.flags,
    };

    let host = opts.host.as_ref().map(|s| s.as_str());
    let port = opts.port.as_ref().map(|p| -> std::borrow::Cow<str> {
        match p {
            Either::A(ref s) => s.as_str().into(),
            Either::B(i) => i.to_string().into(),
        }
    });
    let port = port.as_ref().map(|p| p.as_ref());

    let addrs = dns_lookup::getaddrinfo(host, port, Some(hints))
        .map_err(|err| convert_socket_error(vm, err, SocketError::GaiError))?;

    let list = addrs
        .map(|ai| {
            ai.map(|ai| {
                vm.ctx.new_tuple(vec![
                    vm.ctx.new_int(ai.address),
                    vm.ctx.new_int(ai.socktype),
                    vm.ctx.new_int(ai.protocol),
                    ai.canonname.into_pyobject(vm),
                    get_ip_addr_tuple(&ai.sockaddr, vm),
                ])
            })
        })
        .collect::<io::Result<Vec<_>>>()
        .map_err(|e| e.into_pyexception(vm))?;
    Ok(vm.ctx.new_list(list))
}

fn _socket_gethostbyaddr(
    addr: PyStrRef,
    vm: &VirtualMachine,
) -> PyResult<(String, PyObjectRef, PyObjectRef)> {
    let addr = get_addr(vm, addr, c::AF_UNSPEC)?;
    let (hostname, _) = dns_lookup::getnameinfo(&addr, 0)
        .map_err(|e| convert_socket_error(vm, e, SocketError::HError))?;
    Ok((
        hostname,
        vm.ctx.new_list(vec![]),
        vm.ctx
            .new_list(vec![vm.ctx.new_utf8_str(addr.ip().to_string())]),
    ))
}

fn _socket_gethostbyname(name: PyStrRef, vm: &VirtualMachine) -> PyResult<String> {
    let addr = get_addr(vm, name, c::AF_INET)?;
    match addr {
        SocketAddr::V4(ip) => Ok(ip.ip().to_string()),
        _ => unreachable!(),
    }
}

fn _socket_inet_pton(af_inet: i32, ip_string: PyStrRef, vm: &VirtualMachine) -> PyResult {
    match af_inet {
        c::AF_INET => ip_string
            .as_str()
            .parse::<Ipv4Addr>()
            .map(|ip_addr| vm.ctx.new_bytes(ip_addr.octets().to_vec()))
            .map_err(|_| {
                vm.new_os_error("illegal IP address string passed to inet_pton".to_owned())
            }),
        c::AF_INET6 => ip_string
            .as_str()
            .parse::<Ipv6Addr>()
            .map(|ip_addr| vm.ctx.new_bytes(ip_addr.octets().to_vec()))
            .map_err(|_| {
                vm.new_os_error("illegal IP address string passed to inet_pton".to_owned())
            }),
        _ => Err(vm.new_os_error("Address family not supported by protocol".to_owned())),
    }
}

fn _socket_inet_ntop(
    af_inet: i32,
    packed_ip: ArgBytesLike,
    vm: &VirtualMachine,
) -> PyResult<String> {
    let packed_ip = packed_ip.borrow_buf();
    match af_inet {
        c::AF_INET => {
            let packed_ip = <&[u8; 4]>::try_from(&*packed_ip).map_err(|_| {
                vm.new_value_error("invalid length of packed IP address string".to_owned())
            })?;
            Ok(Ipv4Addr::from(*packed_ip).to_string())
        }
        c::AF_INET6 => {
            let packed_ip = <&[u8; 16]>::try_from(&*packed_ip).map_err(|_| {
                vm.new_value_error("invalid length of packed IP address string".to_owned())
            })?;
            Ok(get_ipv6_addr_str(Ipv6Addr::from(*packed_ip)))
        }
        _ => Err(vm.new_value_error(format!("unknown address family {}", af_inet))),
    }
}

fn _socket_getprotobyname(name: PyStrRef, vm: &VirtualMachine) -> PyResult {
    let cstr = name.to_cstring(vm)?;
    let proto = unsafe { c::getprotobyname(cstr.as_ptr()) };
    if proto.is_null() {
        return Err(vm.new_os_error("protocol not found".to_owned()));
    }
    let num = unsafe { (*proto).p_proto };
    Ok(vm.ctx.new_int(num))
}

fn _socket_getnameinfo(
    address: PyTupleRef,
    flags: i32,
    vm: &VirtualMachine,
) -> PyResult<(String, String)> {
    let address = address.as_slice();
    match address.len() {
        2 | 3 | 4 => {}
        _ => return Err(vm.new_type_error("illegal sockaddr argument".to_owned())),
    }
    let (addr, flowinfo, scopeid) = Address::from_tuple_ipv6(address, vm)?;
    let hints = dns_lookup::AddrInfoHints {
        address: c::AF_UNSPEC,
        socktype: c::SOCK_DGRAM,
        flags: c::AI_NUMERICHOST,
        protocol: 0,
    };
    let service = addr.port.to_string();
    let mut res = dns_lookup::getaddrinfo(Some(addr.host.as_str()), Some(&service), Some(hints))
        .map_err(|e| convert_socket_error(vm, e, SocketError::GaiError))?
        .filter_map(Result::ok);
    let mut ainfo = res.next().unwrap();
    if res.next().is_some() {
        return Err(vm.new_os_error("sockaddr resolved to multiple addresses".to_owned()));
    }
    match &mut ainfo.sockaddr {
        SocketAddr::V4(_) => {
            if address.len() != 2 {
                return Err(vm.new_os_error("IPv4 sockaddr must be 2 tuple".to_owned()));
            }
        }
        SocketAddr::V6(addr) => {
            addr.set_flowinfo(flowinfo);
            addr.set_scope_id(scopeid);
        }
    }
    dns_lookup::getnameinfo(&ainfo.sockaddr, flags)
        .map_err(|e| convert_socket_error(vm, e, SocketError::GaiError))
}

#[cfg(unix)]
fn _socket_socketpair(
    family: OptionalArg<i32>,
    socket_kind: OptionalArg<i32>,
    proto: OptionalArg<i32>,
    vm: &VirtualMachine,
) -> PyResult<(PySocket, PySocket)> {
    let family = family.unwrap_or(libc::AF_UNIX);
    let socket_kind = socket_kind.unwrap_or(libc::SOCK_STREAM);
    let proto = proto.unwrap_or(0);
    let (a, b) = Socket::pair(family.into(), socket_kind.into(), Some(proto.into()))
        .map_err(|e| e.into_pyexception(vm))?;
    let py_a = PySocket::default();
    py_a.init_inner(family, socket_kind, proto, a, vm)?;
    let py_b = PySocket::default();
    py_b.init_inner(family, socket_kind, proto, b, vm)?;
    Ok((py_a, py_b))
}

#[cfg(all(unix, not(target_os = "redox")))]
type IfIndex = c::c_uint;
#[cfg(windows)]
type IfIndex = winapi::shared::ifdef::NET_IFINDEX;

#[cfg(not(target_os = "redox"))]
fn _socket_if_nametoindex(name: PyObjectRef, vm: &VirtualMachine) -> PyResult<IfIndex> {
    let name = crate::vm::stdlib::os::FsPath::try_from(name, true, vm)?;
    let name = ffi::CString::new(name.as_bytes()).map_err(|err| err.into_pyexception(vm))?;

    let ret = unsafe { c::if_nametoindex(name.as_ptr()) };

    if ret == 0 {
        Err(vm.new_os_error("no interface with this name".to_owned()))
    } else {
        Ok(ret)
    }
}

#[cfg(not(target_os = "redox"))]
fn _socket_if_indextoname(index: IfIndex, vm: &VirtualMachine) -> PyResult<String> {
    let mut buf = [0; c::IF_NAMESIZE + 1];
    let ret = unsafe { c::if_indextoname(index, buf.as_mut_ptr()) };
    if ret.is_null() {
        Err(crate::vm::stdlib::os::errno_err(vm))
    } else {
        let buf = unsafe { ffi::CStr::from_ptr(buf.as_ptr()) };
        Ok(buf.to_string_lossy().into_owned())
    }
}

#[cfg(any(
    windows,
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "fuchsia",
    target_os = "ios",
    target_os = "linux",
    target_os = "macos",
    target_os = "netbsd",
    target_os = "openbsd",
))]
fn _socket_if_nameindex(vm: &VirtualMachine) -> PyResult {
    #[cfg(not(windows))]
    {
        let list = if_nameindex()
            .map_err(|err| err.into_pyexception(vm))?
            .to_slice()
            .iter()
            .map(|iface| {
                let tup: (u32, String) =
                    (iface.index(), iface.name().to_string_lossy().into_owned());
                tup.into_pyobject(vm)
            })
            .collect();

        return Ok(vm.ctx.new_list(list));

        // all the stuff below should be in nix soon, hopefully

        use ffi::CStr;
        use std::ptr::NonNull;

        #[repr(transparent)]
        struct Interface(libc::if_nameindex);

        impl Interface {
            fn index(&self) -> libc::c_uint {
                self.0.if_index
            }
            fn name(&self) -> &CStr {
                unsafe { CStr::from_ptr(self.0.if_name) }
            }
        }

        struct Interfaces {
            ptr: NonNull<libc::if_nameindex>,
        }

        impl Interfaces {
            fn to_slice(&self) -> &[Interface] {
                let ifs = self.ptr.as_ptr() as *const Interface;
                let mut len = 0;
                unsafe {
                    while (*ifs.add(len)).0.if_index != 0 {
                        len += 1
                    }
                    std::slice::from_raw_parts(ifs, len)
                }
            }
        }

        impl Drop for Interfaces {
            fn drop(&mut self) {
                unsafe { libc::if_freenameindex(self.ptr.as_ptr()) };
            }
        }

        fn if_nameindex() -> nix::Result<Interfaces> {
            unsafe {
                let ifs = libc::if_nameindex();
                let ptr = NonNull::new(ifs).ok_or_else(nix::Error::last)?;
                Ok(Interfaces { ptr })
            }
        }
    }
    #[cfg(windows)]
    {
        use std::ptr;

        let table = MibTable::get_raw().map_err(|err| err.into_pyexception(vm))?;
        let list = table.as_slice().iter().map(|entry| {
            let name = get_name(&entry.InterfaceLuid).map_err(|err| err.into_pyexception(vm))?;
            let tup = (entry.InterfaceIndex, name.to_string_lossy());
            Ok(tup.into_pyobject(vm))
        });
        let list = list.collect::<PyResult<_>>()?;
        return Ok(vm.ctx.new_list(list));

        fn get_name(luid: &winapi::shared::ifdef::NET_LUID) -> io::Result<widestring::WideCString> {
            let mut buf = [0; c::IF_NAMESIZE + 1];
            let ret =
                unsafe { netioapi::ConvertInterfaceLuidToNameW(luid, buf.as_mut_ptr(), buf.len()) };
            if ret == 0 {
                Ok(widestring::WideCString::from_vec_with_nul(&buf[..]).unwrap())
            } else {
                Err(io::Error::from_raw_os_error(ret as i32))
            }
        }
        struct MibTable {
            ptr: ptr::NonNull<netioapi::MIB_IF_TABLE2>,
        }
        impl MibTable {
            fn get_raw() -> io::Result<Self> {
                let mut ptr = ptr::null_mut();
                let ret = unsafe { netioapi::GetIfTable2Ex(netioapi::MibIfTableRaw, &mut ptr) };
                if ret == 0 {
                    let ptr = unsafe { ptr::NonNull::new_unchecked(ptr) };
                    Ok(Self { ptr })
                } else {
                    Err(io::Error::from_raw_os_error(ret as i32))
                }
            }
        }
        impl MibTable {
            fn as_slice(&self) -> &[netioapi::MIB_IF_ROW2] {
                unsafe {
                    let p = self.ptr.as_ptr();
                    let ptr = ptr::addr_of!((*p).Table) as *const netioapi::MIB_IF_ROW2;
                    std::slice::from_raw_parts(ptr, (*p).NumEntries as usize)
                }
            }
        }
        impl Drop for MibTable {
            fn drop(&mut self) {
                unsafe { netioapi::FreeMibTable(self.ptr.as_ptr() as *mut _) }
            }
        }
    }
}

fn get_addr(vm: &VirtualMachine, pyname: PyStrRef, af: i32) -> PyResult<SocketAddr> {
    let name = pyname.as_str();
    if name.is_empty() {
        let hints = dns_lookup::AddrInfoHints {
            address: af,
            socktype: c::SOCK_DGRAM,
            flags: c::AI_PASSIVE,
            protocol: 0,
        };
        let mut res = dns_lookup::getaddrinfo(None, Some("0"), Some(hints))
            .map_err(|e| convert_socket_error(vm, e, SocketError::GaiError))?;
        let ainfo = res.next().unwrap().map_err(|e| e.into_pyexception(vm))?;
        if res.next().is_some() {
            return Err(vm.new_os_error("wildcard resolved to multiple address".to_owned()));
        }
        return Ok(ainfo.sockaddr);
    }
    if name == "255.255.255.255" || name == "<broadcast>" {
        match af {
            c::AF_INET | c::AF_UNSPEC => {}
            _ => return Err(vm.new_os_error("address family mismatched".to_owned())),
        }
        return Ok(SocketAddr::V4(net::SocketAddrV4::new(
            c::INADDR_BROADCAST.into(),
            0,
        )));
    }
    if let c::AF_INET | c::AF_UNSPEC = af {
        if let Ok(addr) = name.parse::<Ipv4Addr>() {
            return Ok(SocketAddr::V4(net::SocketAddrV4::new(addr, 0)));
        }
    }
    if matches!(af, c::AF_INET | c::AF_UNSPEC) && !name.contains('%') {
        if let Ok(addr) = name.parse::<Ipv6Addr>() {
            return Ok(SocketAddr::V6(net::SocketAddrV6::new(addr, 0, 0, 0)));
        }
    }
    let hints = dns_lookup::AddrInfoHints {
        address: af,
        ..Default::default()
    };
    let name = vm
        .state
        .codec_registry
        .encode_text(pyname, "idna", None, vm)?;
    let name = std::str::from_utf8(name.as_bytes())
        .map_err(|_| vm.new_runtime_error("idna output is not utf8".to_owned()))?;
    let mut res = dns_lookup::getaddrinfo(Some(name), None, Some(hints))
        .map_err(|e| convert_socket_error(vm, e, SocketError::GaiError))?;
    res.next()
        .unwrap()
        .map(|ainfo| ainfo.sockaddr)
        .map_err(|e| e.into_pyexception(vm))
}

fn sock_from_raw(fileno: RawSocket, vm: &VirtualMachine) -> PyResult<Socket> {
    let invalid = {
        cfg_if::cfg_if! {
            if #[cfg(windows)] {
                fileno == INVALID_SOCKET
            } else {
                fileno < 0
            }
        }
    };
    if invalid {
        return Err(vm.new_value_error("negative file descriptor".to_owned()));
    }
    Ok(unsafe { sock_from_raw_unchecked(fileno) })
}
/// SAFETY: fileno must not be equal to INVALID_SOCKET
unsafe fn sock_from_raw_unchecked(fileno: RawSocket) -> Socket {
    #[cfg(unix)]
    {
        use std::os::unix::io::FromRawFd;
        Socket::from_raw_fd(fileno)
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::FromRawSocket;
        Socket::from_raw_socket(fileno)
    }
}
pub(super) fn sock_fileno(sock: &Socket) -> RawSocket {
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

pub(super) const INVALID_SOCKET: RawSocket = {
    #[cfg(unix)]
    {
        -1
    }
    #[cfg(windows)]
    {
        winapi::um::winsock2::INVALID_SOCKET as RawSocket
    }
};

fn convert_socket_error(
    vm: &VirtualMachine,
    err: dns_lookup::LookupError,
    err_kind: SocketError,
) -> PyBaseExceptionRef {
    if let dns_lookup::LookupErrorKind::System = err.kind() {
        return io::Error::from(err).into_pyexception(vm);
    }
    let strerr = {
        #[cfg(unix)]
        {
            let s = match err_kind {
                SocketError::GaiError => unsafe {
                    ffi::CStr::from_ptr(libc::gai_strerror(err.error_num()))
                },
                SocketError::HError => unsafe {
                    ffi::CStr::from_ptr(libc::hstrerror(err.error_num()))
                },
            };
            s.to_str().unwrap()
        }
        #[cfg(windows)]
        {
            "getaddrinfo failed"
        }
    };
    let exception_cls = match err_kind {
        SocketError::GaiError => &GAI_ERROR,
        SocketError::HError => &HERROR,
    };
    vm.new_exception(
        exception_cls.get().unwrap().clone(),
        vec![vm.ctx.new_int(err.error_num()), vm.ctx.new_utf8_str(strerr)],
    )
}

fn timeout_error(vm: &VirtualMachine) -> PyBaseExceptionRef {
    timeout_error_msg(vm, "timed out".to_owned())
}
pub(super) fn timeout_error_msg(vm: &VirtualMachine, msg: String) -> PyBaseExceptionRef {
    vm.new_exception_msg(TIMEOUT_ERROR.get().unwrap().clone(), msg)
}

fn get_ipv6_addr_str(ipv6: Ipv6Addr) -> String {
    match ipv6.to_ipv4() {
        // instead of "::0.0.ddd.ddd" it's "::xxxx"
        Some(v4) if !ipv6.is_unspecified() && matches!(v4.octets(), [0, 0, _, _]) => {
            format!("::{:x}", u32::from(v4))
        }
        _ => ipv6.to_string(),
    }
}

pub(crate) struct Deadline {
    deadline: Instant,
}

impl Deadline {
    fn new(timeout: Duration) -> Self {
        Self {
            deadline: Instant::now() + timeout,
        }
    }
    fn time_until(&self) -> Result<Duration, IoOrPyException> {
        self.deadline
            .checked_duration_since(Instant::now())
            // past the deadline already
            .ok_or(IoOrPyException::Timeout)
    }
}

static DEFAULT_TIMEOUT: AtomicCell<f64> = AtomicCell::new(-1.0);

fn _socket_getdefaulttimeout() -> Option<f64> {
    let timeout = DEFAULT_TIMEOUT.load();
    if timeout >= 0.0 {
        Some(timeout)
    } else {
        None
    }
}

fn _socket_setdefaulttimeout(timeout: Option<Duration>) {
    DEFAULT_TIMEOUT.store(timeout.map_or(-1.0, |d| d.as_secs_f64()));
}

fn _socket_dup(x: PyObjectRef, vm: &VirtualMachine) -> PyResult<RawSocket> {
    let sock = get_raw_sock(x, vm)?;
    let sock = std::mem::ManuallyDrop::new(sock_from_raw(sock, vm)?);
    let newsock = sock.try_clone().map_err(|e| e.into_pyexception(vm))?;
    let fd = into_sock_fileno(newsock);
    #[cfg(windows)]
    crate::vm::stdlib::nt::raw_set_handle_inheritable(fd as _, false)
        .map_err(|e| e.into_pyexception(vm))?;
    Ok(fd)
}

fn _socket_close(x: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
    close_inner(get_raw_sock(x, vm)?, vm)
}

fn close_inner(x: RawSocket, vm: &VirtualMachine) -> PyResult<()> {
    #[cfg(unix)]
    use libc::close;
    #[cfg(windows)]
    use winapi::um::winsock2::closesocket as close;
    let ret = unsafe { close(x as _) };
    if ret < 0 {
        let err = crate::vm::stdlib::os::errno();
        if err.raw_os_error() != Some(errcode!(ECONNRESET)) {
            return Err(err.into_pyexception(vm));
        }
    }
    Ok(())
}

enum SocketError {
    HError,
    GaiError,
}

rustpython_common::static_cell! {
    static TIMEOUT_ERROR: PyTypeRef;
    static HERROR: PyTypeRef;
    static GAI_ERROR: PyTypeRef;
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    #[cfg(windows)]
    crate::vm::stdlib::nt::init_winsock();

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
    let socket_herror = HERROR
        .get_or_init(|| {
            ctx.new_class(
                "socket.herror",
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

    let socket = PySocket::make_class(ctx);
    let module = py_module!(vm, "_socket", {
        "socket" => socket.clone(),
        "SocketType" => socket,
        "error" => ctx.exceptions.os_error.clone(),
        "timeout" => socket_timeout,
        "herror" => socket_herror,
        "gaierror" => socket_gaierror,
        "inet_aton" => named_function!(ctx, _socket, inet_aton),
        "inet_ntoa" => named_function!(ctx, _socket, inet_ntoa),
        "gethostname" => named_function!(ctx, _socket, gethostname),
        "htonl" => ctx.new_function("htonl", u32::to_be),
        "htons" => ctx.new_function("htons", u16::to_be),
        "ntohl" => ctx.new_function("ntohl", u32::from_be),
        "ntohs" => ctx.new_function("ntohs", u16::from_be),
        "getdefaulttimeout" => named_function!(ctx, _socket, getdefaulttimeout),
        "setdefaulttimeout" => named_function!(ctx, _socket, setdefaulttimeout),
        "has_ipv6" => ctx.new_bool(true),
        "inet_pton" => named_function!(ctx, _socket, inet_pton),
        "inet_ntop" => named_function!(ctx, _socket, inet_ntop),
        "getprotobyname" => named_function!(ctx, _socket, getprotobyname),
        "getservbyname" => named_function!(ctx, _socket, getservbyname),
        "getservbyport" => named_function!(ctx, _socket, getservbyport),
        "dup" => named_function!(ctx, _socket, dup),
        "close" => named_function!(ctx, _socket, close),
        "getaddrinfo" => named_function!(ctx, _socket, getaddrinfo),
        "gethostbyaddr" => named_function!(ctx, _socket, gethostbyaddr),
        "gethostbyname" => named_function!(ctx, _socket, gethostbyname),
        "getnameinfo" => named_function!(ctx, _socket, getnameinfo),
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
        "SOL_TCP" => ctx.new_int(6),
        "SO_REUSEADDR" => ctx.new_int(c::SO_REUSEADDR),
        "SO_TYPE" => ctx.new_int(c::SO_TYPE),
        "SO_BROADCAST" => ctx.new_int(c::SO_BROADCAST),
        "SO_OOBINLINE" => ctx.new_int(c::SO_OOBINLINE),
        "SO_ERROR" => ctx.new_int(c::SO_ERROR),
        "SO_LINGER" => ctx.new_int(c::SO_LINGER),
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

    #[cfg(not(target_os = "redox"))]
    extend_module!(vm, module, {
        "if_nametoindex" => named_function!(ctx, _socket, if_nametoindex),
        "if_indextoname" => named_function!(ctx, _socket, if_indextoname),
        "SOCK_RAW" => ctx.new_int(c::SOCK_RAW),
        "SOCK_RDM" => ctx.new_int(c::SOCK_RDM),
        "SOCK_SEQPACKET" => ctx.new_int(c::SOCK_SEQPACKET),
    });

    #[cfg(any(
        windows,
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "fuchsia",
        target_os = "ios",
        target_os = "linux",
        target_os = "macos",
        target_os = "netbsd",
        target_os = "openbsd",
    ))]
    extend_module!(vm, module, {
        "if_nameindex" => named_function!(ctx, _socket, if_nameindex),
    });

    extend_module_platform_specific(vm, &module);

    module
}

#[cfg(windows)]
fn extend_module_platform_specific(vm: &VirtualMachine, module: &PyObjectRef) {
    let ctx = &vm.ctx;
    extend_module!(vm, module, {
        "IPPROTO_ICLFXBM" => ctx.new_int(c::IPPROTO_ICLFXBM),
        "IPPROTO_ST" => ctx.new_int(c::IPPROTO_ST),
        "IPPROTO_CBT" => ctx.new_int(c::IPPROTO_CBT),
        "IPPROTO_IGP" => ctx.new_int(c::IPPROTO_IGP),
        "IPPROTO_RDP" => ctx.new_int(c::IPPROTO_RDP),
        "IPPROTO_PGM" => ctx.new_int(c::IPPROTO_PGM),
        "IPPROTO_L2TP" => ctx.new_int(c::IPPROTO_L2TP),
        "IPPROTO_SCTP" => ctx.new_int(c::IPPROTO_SCTP),
    });
}

#[cfg(unix)]
fn extend_module_platform_specific(vm: &VirtualMachine, module: &PyObjectRef) {
    let ctx = &vm.ctx;

    extend_module!(vm, module, {
        "socketpair" => named_function!(ctx, _socket, socketpair),
        "AF_UNIX" => ctx.new_int(c::AF_UNIX),
        "SO_REUSEPORT" => ctx.new_int(c::SO_REUSEPORT),
    });

    #[cfg(not(target_os = "redox"))]
    extend_module!(vm, module, {
        "sethostname" => named_function!(ctx, _socket, sethostname),
    });
}
