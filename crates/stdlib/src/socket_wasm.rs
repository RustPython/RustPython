//! Wasm `_socket` surface so `Lib/socket.py` and `Lib/ssl.py` can import.
//!
//! There are no BSD sockets on wasm32-unknown-unknown, and WASI does not
//! expose the host_env socket engine yet. Constants, address conversion,
//! timeouts, and a constructible `socket` type are provided; connect-side
//! operations raise `OSError`.

pub(crate) use _socket::module_def;

#[pymodule]
mod _socket {
    use rustpython_vm::{
        Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
        builtins::{PyType, PyTypeRef, PyUtf8StrRef},
        common::lock::PyMutex,
        function::{ArgBytesLike, ArgIntoFloat, OptionalArg},
        types::{Constructor, Initializer},
    };
    use std::net::{Ipv4Addr, Ipv6Addr};
    use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};

    #[pyattr]
    const AF_UNSPEC: i32 = 0;
    #[pyattr]
    const AF_UNIX: i32 = 1;
    #[pyattr]
    const AF_INET: i32 = 2;
    #[pyattr]
    const AF_INET6: i32 = 10;

    #[pyattr]
    const SOCK_STREAM: i32 = 1;
    #[pyattr]
    const SOCK_DGRAM: i32 = 2;
    #[pyattr]
    const SOCK_RAW: i32 = 3;

    #[pyattr]
    const SOL_SOCKET: i32 = 1;
    #[pyattr]
    const SO_REUSEADDR: i32 = 2;
    #[pyattr]
    const SO_TYPE: i32 = 3;
    #[pyattr]
    const SO_ERROR: i32 = 4;
    #[pyattr]
    const SO_BROADCAST: i32 = 6;
    #[pyattr]
    const SO_KEEPALIVE: i32 = 9;
    #[pyattr]
    const SO_RCVBUF: i32 = 8;
    #[pyattr]
    const SO_SNDBUF: i32 = 7;

    #[pyattr]
    const IPPROTO_IP: i32 = 0;
    #[pyattr]
    const IPPROTO_TCP: i32 = 6;
    #[pyattr]
    const IPPROTO_UDP: i32 = 17;
    #[pyattr]
    const IPPROTO_IPV6: i32 = 41;
    #[pyattr]
    const SOL_TCP: i32 = IPPROTO_TCP;

    #[pyattr]
    const SHUT_RD: i32 = 0;
    #[pyattr]
    const SHUT_WR: i32 = 1;
    #[pyattr]
    const SHUT_RDWR: i32 = 2;

    #[pyattr]
    const MSG_OOB: i32 = 1;
    #[pyattr]
    const MSG_PEEK: i32 = 2;
    #[pyattr]
    const MSG_DONTROUTE: i32 = 4;

    #[pyattr]
    const AI_PASSIVE: i32 = 1;
    #[pyattr]
    const AI_CANONNAME: i32 = 2;
    #[pyattr]
    const AI_NUMERICHOST: i32 = 4;
    #[pyattr]
    const AI_NUMERICSERV: i32 = 8;
    #[pyattr]
    const AI_ADDRCONFIG: i32 = 32;

    #[pyattr]
    const NI_NUMERICHOST: i32 = 1;
    #[pyattr]
    const NI_NUMERICSERV: i32 = 2;
    #[pyattr]
    const NI_NOFQDN: i32 = 4;
    #[pyattr]
    const NI_NAMEREQD: i32 = 8;
    #[pyattr]
    const NI_DGRAM: i32 = 16;

    #[pyattr]
    const INADDR_ANY: u32 = 0;
    #[pyattr]
    const INADDR_LOOPBACK: u32 = 0x7f00_0001;
    #[pyattr]
    const INADDR_BROADCAST: u32 = 0xffff_ffff;
    #[pyattr]
    const INADDR_NONE: u32 = 0xffff_ffff;
    #[pyattr]
    const IPPORT_RESERVED: i32 = 1024;
    #[pyattr]
    const IPPORT_USERRESERVED: i32 = 5000;
    #[pyattr]
    const TCP_NODELAY: i32 = 1;
    #[pyattr(name = "has_ipv6")]
    const HAS_IPV6: bool = true;

    static DEFAULT_TIMEOUT: AtomicU64 = AtomicU64::new(f64::to_bits(-1.0));

    fn unsupported(
        vm: &VirtualMachine,
        op: &str,
    ) -> PyRef<rustpython_vm::builtins::PyBaseException> {
        vm.new_os_error(format!("{op} is not available"))
    }

    #[pyattr]
    fn error(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.exceptions.os_error.to_owned()
    }

    #[pyattr]
    fn timeout(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.exceptions.timeout_error.to_owned()
    }

    #[pyattr(once)]
    fn herror(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.new_exception_type(
            "socket",
            "herror",
            Some(vec![vm.ctx.exceptions.os_error.to_owned()]),
        )
    }

    #[pyattr(once)]
    fn gaierror(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.new_exception_type(
            "socket",
            "gaierror",
            Some(vec![vm.ctx.exceptions.os_error.to_owned()]),
        )
    }

    #[pyfunction]
    const fn htonl(x: u32) -> u32 {
        u32::to_be(x)
    }

    #[pyfunction]
    const fn htons(x: u16) -> u16 {
        u16::to_be(x)
    }

    #[pyfunction]
    const fn ntohl(x: u32) -> u32 {
        u32::from_be(x)
    }

    #[pyfunction]
    const fn ntohs(x: u16) -> u16 {
        u16::from_be(x)
    }

    #[pyfunction]
    fn inet_aton(ip: PyUtf8StrRef, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        ip.as_str()
            .parse::<Ipv4Addr>()
            .map(|addr| addr.octets().to_vec())
            .map_err(|_| vm.new_os_error("illegal IP address string passed to inet_aton"))
    }

    #[pyfunction]
    fn inet_ntoa(packed: ArgBytesLike, vm: &VirtualMachine) -> PyResult<String> {
        let buf = packed.borrow_buf();
        let octets: [u8; 4] = (&*buf)
            .try_into()
            .map_err(|_| vm.new_os_error("packed IP wrong length for inet_ntoa"))?;
        Ok(Ipv4Addr::from(octets).to_string())
    }

    #[pyfunction]
    fn inet_pton(af: i32, ip: PyUtf8StrRef, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        match af {
            AF_INET => ip
                .as_str()
                .parse::<Ipv4Addr>()
                .map(|addr| addr.octets().to_vec())
                .map_err(|_| vm.new_os_error("illegal IP address string passed to inet_pton")),
            AF_INET6 => ip
                .as_str()
                .parse::<Ipv6Addr>()
                .map(|addr| addr.octets().to_vec())
                .map_err(|_| vm.new_os_error("illegal IP address string passed to inet_pton")),
            _ => Err(vm.new_os_error("Address family not supported")),
        }
    }

    #[pyfunction]
    fn inet_ntop(af: i32, packed: ArgBytesLike, vm: &VirtualMachine) -> PyResult<String> {
        let buf = packed.borrow_buf();
        match af {
            AF_INET => {
                let octets: [u8; 4] = (&*buf).try_into().map_err(|_| {
                    vm.new_value_error("invalid length of packed IP address string")
                })?;
                Ok(Ipv4Addr::from(octets).to_string())
            }
            AF_INET6 => {
                let octets: [u8; 16] = (&*buf).try_into().map_err(|_| {
                    vm.new_value_error("invalid length of packed IP address string")
                })?;
                Ok(Ipv6Addr::from(octets).to_string())
            }
            _ => Err(vm.new_value_error("unknown address family")),
        }
    }

    #[pyfunction]
    fn getdefaulttimeout() -> Option<f64> {
        let timeout = f64::from_bits(DEFAULT_TIMEOUT.load(Ordering::Relaxed));
        (timeout >= 0.0).then_some(timeout)
    }

    #[pyfunction]
    fn setdefaulttimeout(timeout: Option<ArgIntoFloat>, vm: &VirtualMachine) -> PyResult<()> {
        match timeout {
            None => DEFAULT_TIMEOUT.store((-1.0f64).to_bits(), Ordering::Relaxed),
            Some(value) => {
                let value = value.into_float();
                if value.is_nan() {
                    return Err(vm.new_value_error("Invalid value NaN (not a number)"));
                }
                if value < 0.0 || !value.is_finite() {
                    return Err(vm.new_value_error("Timeout value out of range"));
                }
                DEFAULT_TIMEOUT.store(value.to_bits(), Ordering::Relaxed);
            }
        }
        Ok(())
    }

    #[pyfunction]
    fn gethostname(vm: &VirtualMachine) -> PyResult<String> {
        Err(unsupported(vm, "gethostname").into())
    }

    #[pyfunction]
    fn gethostbyname(_name: PyUtf8StrRef, vm: &VirtualMachine) -> PyResult<String> {
        Err(unsupported(vm, "gethostbyname").into())
    }

    #[pyfunction]
    fn getaddrinfo(
        _host: OptionalArg<PyObjectRef>,
        _port: OptionalArg<PyObjectRef>,
        _family: OptionalArg<i32>,
        _type: OptionalArg<i32>,
        _proto: OptionalArg<i32>,
        _flags: OptionalArg<i32>,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<PyObjectRef>> {
        Err(unsupported(vm, "getaddrinfo").into())
    }

    #[derive(FromArgs)]
    struct SocketInitArgs {
        #[pyarg(any, optional)]
        family: OptionalArg<i32>,
        #[pyarg(any, optional)]
        r#type: OptionalArg<i32>,
        #[pyarg(any, optional)]
        proto: OptionalArg<i32>,
        #[pyarg(any, optional)]
        fileno: OptionalArg<Option<PyObjectRef>>,
    }

    #[pyattr(name = "socket")]
    #[pyattr(name = "SocketType")]
    #[pyclass(name = "socket")]
    #[derive(Debug, PyPayload)]
    struct PySocket {
        family: AtomicI32,
        kind: AtomicI32,
        proto: AtomicI32,
        timeout: PyMutex<Option<f64>>,
        closed: PyMutex<bool>,
    }

    impl Default for PySocket {
        fn default() -> Self {
            Self {
                family: AtomicI32::new(AF_INET),
                kind: AtomicI32::new(SOCK_STREAM),
                proto: AtomicI32::new(0),
                timeout: PyMutex::new(None),
                closed: PyMutex::new(false),
            }
        }
    }

    #[pyclass(with(Constructor, Initializer), flags(BASETYPE))]
    impl PySocket {
        fn ensure_open(&self, vm: &VirtualMachine) -> PyResult<()> {
            if *self.closed.lock() {
                return Err(vm.new_os_error("Bad file descriptor"));
            }
            Ok(())
        }

        #[pygetset]
        fn family(&self) -> i32 {
            self.family.load(Ordering::Relaxed)
        }

        #[pygetset]
        fn r#type(&self) -> i32 {
            self.kind.load(Ordering::Relaxed)
        }

        #[pygetset]
        fn proto(&self) -> i32 {
            self.proto.load(Ordering::Relaxed)
        }

        #[pymethod]
        fn fileno(&self) -> i32 {
            if *self.closed.lock() { -1 } else { 0 }
        }

        #[pymethod]
        fn close(&self) {
            *self.closed.lock() = true;
        }

        #[pymethod]
        fn detach(&self) -> i32 {
            *self.closed.lock() = true;
            -1
        }

        #[pymethod]
        fn gettimeout(&self) -> Option<f64> {
            *self.timeout.lock()
        }

        #[pymethod]
        fn settimeout(&self, timeout: Option<ArgIntoFloat>, vm: &VirtualMachine) -> PyResult<()> {
            *self.timeout.lock() = match timeout {
                None => None,
                Some(value) => {
                    let value = value.into_float();
                    if value.is_nan() {
                        return Err(vm.new_value_error("Invalid value NaN (not a number)"));
                    }
                    if value < 0.0 || !value.is_finite() {
                        return Err(vm.new_value_error("Timeout value out of range"));
                    }
                    Some(value)
                }
            };
            Ok(())
        }

        #[pymethod]
        fn setblocking(&self, blocking: bool) {
            *self.timeout.lock() = if blocking { None } else { Some(0.0) };
        }

        #[pymethod]
        fn getblocking(&self) -> bool {
            !matches!(*self.timeout.lock(), Some(t) if t == 0.0)
        }

        #[pymethod]
        fn getsockopt(&self, level: i32, optname: i32, vm: &VirtualMachine) -> PyResult<i32> {
            self.ensure_open(vm)?;
            if level == SOL_SOCKET && optname == SO_TYPE {
                return Ok(self.kind.load(Ordering::Relaxed));
            }
            if level == SOL_SOCKET && optname == SO_ERROR {
                return Ok(0);
            }
            Err(unsupported(vm, "getsockopt").into())
        }

        #[pymethod]
        fn setsockopt(
            &self,
            _level: i32,
            _optname: i32,
            _value: OptionalArg<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            self.ensure_open(vm)?;
            Ok(())
        }

        #[pymethod]
        fn bind(&self, _address: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            self.ensure_open(vm)?;
            Err(unsupported(vm, "bind").into())
        }

        #[pymethod]
        fn connect(&self, _address: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            self.ensure_open(vm)?;
            Err(unsupported(vm, "connect").into())
        }

        #[pymethod]
        fn listen(&self, _backlog: OptionalArg<i32>, vm: &VirtualMachine) -> PyResult<()> {
            self.ensure_open(vm)?;
            Err(unsupported(vm, "listen").into())
        }

        #[pymethod]
        fn _accept(&self, vm: &VirtualMachine) -> PyResult<(PyObjectRef, PyObjectRef)> {
            self.ensure_open(vm)?;
            Err(unsupported(vm, "accept").into())
        }

        #[pymethod]
        fn send(&self, _data: ArgBytesLike, vm: &VirtualMachine) -> PyResult<usize> {
            self.ensure_open(vm)?;
            Err(unsupported(vm, "send").into())
        }

        #[pymethod]
        fn recv(&self, _bufsize: i32, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            self.ensure_open(vm)?;
            Err(unsupported(vm, "recv").into())
        }

        #[pymethod]
        fn shutdown(&self, _how: i32, vm: &VirtualMachine) -> PyResult<()> {
            self.ensure_open(vm)?;
            Err(unsupported(vm, "shutdown").into())
        }

        #[pymethod]
        fn getsockname(&self, vm: &VirtualMachine) -> PyResult<(String, i32)> {
            self.ensure_open(vm)?;
            Ok(("0.0.0.0".to_owned(), 0))
        }

        #[pymethod]
        fn getpeername(&self, vm: &VirtualMachine) -> PyResult<(String, i32)> {
            self.ensure_open(vm)?;
            Err(unsupported(vm, "getpeername").into())
        }
    }

    impl Constructor for PySocket {
        type Args = ();

        fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
            Ok(Self::default())
        }
    }

    impl Initializer for PySocket {
        type Args = SocketInitArgs;

        fn init(zelf: PyRef<Self>, args: Self::Args, _vm: &VirtualMachine) -> PyResult<()> {
            let family = args.family.unwrap_or(AF_INET);
            let kind = args.r#type.unwrap_or(SOCK_STREAM);
            let proto = args.proto.unwrap_or(0);
            let _ = args.fileno;
            zelf.family.store(family, Ordering::Relaxed);
            zelf.kind.store(kind, Ordering::Relaxed);
            zelf.proto.store(proto, Ordering::Relaxed);
            *zelf.closed.lock() = false;
            let default = f64::from_bits(DEFAULT_TIMEOUT.load(Ordering::Relaxed));
            *zelf.timeout.lock() = (default >= 0.0).then_some(default);
            Ok(())
        }
    }
}
