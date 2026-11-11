// spell-checker:ignore WASIX
//! Socket support for WebAssembly. The optional WASIX host backend provides
//! real TCP/UDP sockets on WASI preview1; other hosts reject socket creation.

pub(crate) use _socket::module_def;

#[pymodule]
mod _socket {
    use core::{
        net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
        sync::atomic::{AtomicI32, AtomicU64, Ordering},
        time::Duration,
    };
    use rustpython_host_env::{
        errno::errors as errno,
        socket::{Socket, resolve},
    };
    use rustpython_vm::{
        Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
        builtins::{PyBytesRef, PyTupleRef, PyTypeRef, PyUtf8StrRef},
        common::lock::PyMutex,
        convert::{ToPyException, ToPyObject, TryFromObject},
        function::{
            ArgBytesLike, ArgIntoFloat, ArgMemoryBuffer, ArgStrOrBytesLike, FuncArgs, OptionalArg,
        },
        types::{Constructor, DefaultConstructor, Initializer},
    };
    use std::{io, time::Instant};

    #[pyattr]
    use rustpython_host_env::socket::{
        AF_INET, AF_INET6, AF_UNIX, AF_UNSPEC, AI_ADDRCONFIG, AI_CANONNAME, AI_NUMERICHOST,
        AI_NUMERICSERV, AI_PASSIVE, INADDR_ANY, INADDR_BROADCAST, INADDR_LOOPBACK, INADDR_NONE,
        IPPORT_RESERVED, IPPORT_USERRESERVED, IPPROTO_IP, IPPROTO_IPV6, IPPROTO_TCP, IPPROTO_UDP,
        MSG_DONTROUTE, MSG_OOB, MSG_PEEK, NI_DGRAM, NI_NAMEREQD, NI_NOFQDN, NI_NUMERICHOST,
        NI_NUMERICSERV, SHUT_RD, SHUT_RDWR, SHUT_WR, SO_BROADCAST, SO_ERROR, SO_KEEPALIVE,
        SO_RCVBUF, SO_REUSEADDR, SO_SNDBUF, SO_TYPE, SOCK_DGRAM, SOCK_RAW, SOCK_STREAM, SOL_SOCKET,
        SOL_TCP, TCP_NODELAY,
    };
    #[pyattr(name = "has_ipv6")]
    const HAS_IPV6: bool = true;

    static DEFAULT_TIMEOUT: AtomicU64 = AtomicU64::new(f64::to_bits(-1.0));

    fn unsupported(
        vm: &VirtualMachine,
        op: &str,
    ) -> PyRef<rustpython_vm::builtins::PyBaseException> {
        vm.new_exception(
            vm.ctx.exceptions.os_error.to_owned(),
            vec![
                vm.ctx.new_int(errno::ENOTSUP).into(),
                vm.ctx
                    .new_str(format!("{op} is not available on this host"))
                    .into(),
            ],
        )
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

    fn parse_timeout(value: Option<ArgIntoFloat>, vm: &VirtualMachine) -> PyResult<Option<f64>> {
        let Some(value) = value else { return Ok(None) };
        let value = value.into_float();
        if value.is_nan() {
            return Err(vm.new_value_error("Invalid value NaN (not a number)"));
        }
        if value < 0.0 {
            return Err(vm.new_value_error("Timeout value out of range"));
        }
        if Duration::try_from_secs_f64(value).is_err() || value > i64::MAX as f64 / 1e9 {
            return Err(vm.new_overflow_error("timestamp out of range for platform time_t"));
        }
        Ok(Some(value))
    }

    #[pyfunction(signature_from_flags)]
    fn getdefaulttimeout() -> Option<f64> {
        let timeout = f64::from_bits(DEFAULT_TIMEOUT.load(Ordering::Relaxed));
        (timeout >= 0.0).then_some(timeout)
    }

    #[pyfunction(signature_from_flags)]
    fn setdefaulttimeout(value: Option<ArgIntoFloat>, vm: &VirtualMachine) -> PyResult<()> {
        DEFAULT_TIMEOUT.store(
            parse_timeout(value, vm)?.unwrap_or(-1.0).to_bits(),
            Ordering::Relaxed,
        );
        Ok(())
    }

    #[pyfunction(signature_from_flags)]
    fn gethostname(vm: &VirtualMachine) -> PyResult<String> {
        Err(unsupported(vm, "gethostname"))
    }

    #[pyfunction]
    fn gethostbyaddr(_name: ArgStrOrBytesLike, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        // Keep reverse lookup failure an OSError so socket.getfqdn can use
        // its documented fallback to the supplied name.
        Err(unsupported(vm, "gethostbyaddr"))
    }

    fn hostname(value: ArgStrOrBytesLike, vm: &VirtualMachine) -> PyResult<String> {
        let name = match value {
            ArgStrOrBytesLike::Str(s) => {
                if let Ok(s) = s.as_wtf8().as_str()
                    && s.is_ascii()
                {
                    s.to_owned()
                } else {
                    let encoded = vm.state.codec_registry.encode_text(s, "idna", None, vm)?;
                    String::from_utf8(encoded.as_bytes().to_vec()).map_err(|_| {
                        vm.new_exception_msg(
                            vm.ctx.exceptions.unicode_error.to_owned(),
                            "invalid IDNA hostname".into(),
                        )
                    })?
                }
            }
            ArgStrOrBytesLike::Buf(b) => core::str::from_utf8(&b.borrow_buf())
                .map(str::to_owned)
                .map_err(|_| {
                vm.new_exception_msg(
                    vm.ctx.exceptions.unicode_error.to_owned(),
                    "hostname is not UTF-8".into(),
                )
            })?,
        };
        if name.contains('\0') {
            return Err(vm.new_type_error("host name must not contain null character"));
        }
        Ok(name)
    }

    fn gai_error(
        code: i32,
        message: &str,
        vm: &VirtualMachine,
    ) -> rustpython_vm::builtins::PyBaseExceptionRef {
        vm.new_exception(
            gaierror(vm),
            vec![vm.ctx.new_int(code).into(), vm.ctx.new_str(message).into()],
        )
    }

    #[pyattr]
    const EAI_NONAME: i32 = -2;
    #[pyattr]
    const EAI_FAMILY: i32 = -6;
    #[pyattr]
    const EAI_SOCKTYPE: i32 = -7;
    #[pyattr]
    const EAI_SERVICE: i32 = -8;
    #[pyattr]
    const EAI_BADFLAGS: i32 = -1;
    #[pyattr]
    const EAI_FAIL: i32 = -4;

    fn addresses(
        host: &str,
        port: u16,
        family: i32,
        numeric: bool,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<SocketAddr>> {
        if !matches!(family, AF_UNSPEC | AF_INET | AF_INET6) {
            return Err(gai_error(EAI_FAMILY, "Address family not supported", vm));
        }
        let all = if let Ok(ip) = host.parse::<IpAddr>() {
            vec![SocketAddr::new(ip, port)]
        } else if numeric {
            return Err(gai_error(EAI_NONAME, "Name or service not known", vm));
        } else {
            resolve(host, port).map_err(|err| gai_error(EAI_NONAME, &err.to_string(), vm))?
        };
        let filtered: Vec<_> = all
            .into_iter()
            .filter(|addr| family == AF_UNSPEC || addr.is_ipv4() == (family == AF_INET))
            .collect();
        if filtered.is_empty() {
            return Err(gai_error(EAI_NONAME, "Name or service not known", vm));
        }
        Ok(filtered)
    }

    fn addr_object(addr: SocketAddr, vm: &VirtualMachine) -> PyObjectRef {
        match addr {
            SocketAddr::V4(addr) => (addr.ip().to_string(), addr.port()).to_pyobject(vm),
            SocketAddr::V6(addr) => (
                addr.ip().to_string(),
                addr.port(),
                addr.flowinfo(),
                addr.scope_id(),
            )
                .to_pyobject(vm),
        }
    }

    #[pyfunction]
    fn gethostbyname(name: ArgStrOrBytesLike, vm: &VirtualMachine) -> PyResult<String> {
        let name = hostname(name, vm)?;
        Ok(addresses(
            if name.is_empty() { "0.0.0.0" } else { &name },
            0,
            AF_INET,
            false,
            vm,
        )?[0]
            .ip()
            .to_string())
    }

    #[derive(FromArgs)]
    struct GaiArgs {
        #[pyarg(any)]
        host: Option<ArgStrOrBytesLike>,
        #[pyarg(any)]
        port: Option<PyObjectRef>,
        #[pyarg(any, default = AF_UNSPEC)]
        family: i32,
        #[pyarg(any, default)]
        r#type: i32,
        #[pyarg(any, default)]
        proto: i32,
        #[pyarg(any, default)]
        flags: i32,
    }

    #[pyfunction]
    fn getaddrinfo(args: GaiArgs, vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
        if args.flags & !(AI_PASSIVE | AI_NUMERICHOST | AI_NUMERICSERV) != 0 {
            return Err(gai_error(EAI_BADFLAGS, "Unsupported getaddrinfo flags", vm));
        }
        if !matches!(args.family, AF_UNSPEC | AF_INET | AF_INET6) {
            return Err(gai_error(EAI_FAMILY, "Address family not supported", vm));
        }
        let port = match args.port {
            None => 0,
            Some(obj) => {
                let n = if let Some(s) = obj.downcast_ref::<rustpython_vm::builtins::PyStr>() {
                    s.as_wtf8()
                        .as_str()
                        .ok()
                        .and_then(|s| s.parse::<i32>().ok())
                        .ok_or_else(|| {
                            gai_error(EAI_SERVICE, "Only numeric service names are supported", vm)
                        })?
                } else {
                    i32::try_from_object(vm, obj)?
                };
                u16::try_from(n).map_err(|_| vm.new_overflow_error("port must be 0-65535"))?
            }
        };
        let kinds: Vec<(i32, i32)> = [(SOCK_STREAM, IPPROTO_TCP), (SOCK_DGRAM, IPPROTO_UDP)]
            .into_iter()
            .filter(|&(kind, proto)| {
                (args.r#type == 0 || args.r#type == kind)
                    && (args.proto == 0 || args.proto == proto)
            })
            .collect();
        if kinds.is_empty() {
            return Err(gai_error(
                EAI_SOCKTYPE,
                "Socket type or protocol not supported",
                vm,
            ));
        }
        let hosts = if let Some(host) = args.host {
            addresses(
                &hostname(host, vm)?,
                port,
                args.family,
                args.flags & AI_NUMERICHOST != 0,
                vm,
            )?
        } else {
            let passive = args.flags & AI_PASSIVE != 0;
            let v4 = if passive {
                Ipv4Addr::UNSPECIFIED
            } else {
                Ipv4Addr::LOCALHOST
            };
            let v6 = if passive {
                Ipv6Addr::UNSPECIFIED
            } else {
                Ipv6Addr::LOCALHOST
            };
            [
                SocketAddr::new(v4.into(), port),
                SocketAddr::new(v6.into(), port),
            ]
            .into_iter()
            .filter(|addr| args.family == AF_UNSPEC || addr.is_ipv4() == (args.family == AF_INET))
            .collect()
        };
        Ok(hosts
            .into_iter()
            .flat_map(|addr| {
                kinds.iter().map(move |&(kind, proto)| {
                    (
                        if addr.is_ipv4() { AF_INET } else { AF_INET6 },
                        kind,
                        proto,
                        "",
                        addr_object(addr, vm),
                    )
                        .to_pyobject(vm)
                })
            })
            .collect())
    }

    #[derive(FromArgs)]
    struct SocketInitArgs {
        #[pyarg(any, default = -1)]
        family: i32,
        #[pyarg(any, default = -1)]
        r#type: i32,
        #[pyarg(any, default = -1)]
        proto: i32,
        #[pyarg(any, optional)]
        fileno: Option<i32>,
    }

    #[derive(FromArgs)]
    struct RecvIntoArgs {
        #[pyarg(any)]
        buffer: ArgMemoryBuffer,
        #[pyarg(any, default)]
        nbytes: isize,
        #[pyarg(any, default)]
        flags: i32,
    }

    #[pyattr(name = "socket")]
    #[pyattr(name = "SocketType")]
    #[pyclass(name = "socket")]
    #[derive(Debug, PyPayload)]
    struct PySocket {
        #[pymember]
        family: AtomicI32,
        #[pymember(name = "type")]
        kind: AtomicI32,
        #[pymember]
        proto: AtomicI32,
        timeout: PyMutex<Option<f64>>,
        socket: PyMutex<Option<Socket>>,
    }

    impl Default for PySocket {
        fn default() -> Self {
            Self {
                family: AtomicI32::new(AF_INET),
                kind: AtomicI32::new(SOCK_STREAM),
                proto: AtomicI32::new(0),
                timeout: PyMutex::new(None),
                socket: PyMutex::new(None),
            }
        }
    }

    impl DefaultConstructor for PySocket {}

    impl Initializer for PySocket {
        type Args = SocketInitArgs;
        fn init(zelf: &Py<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
            let mut family = args.family;
            let mut kind = args.r#type;
            let mut proto = args.proto;
            let socket = if let Some(fd) = args.fileno {
                if fd < 0 {
                    return Err(vm.new_value_error("negative file descriptor"));
                }
                // Ownership transfers only after initialization succeeds.
                let sock = core::mem::ManuallyDrop::new(
                    unsafe { Socket::from_raw_fd(fd) }.map_err(|e| e.to_pyexception(vm))?,
                );
                if family == -1 {
                    family = if sock
                        .local_addr()
                        .map_err(|e| e.to_pyexception(vm))?
                        .is_ipv4()
                    {
                        AF_INET
                    } else {
                        AF_INET6
                    };
                }
                if kind == -1 {
                    kind = sock
                        .get_option(SOL_SOCKET, SO_TYPE)
                        .map_err(|e| e.to_pyexception(vm))?;
                }
                if proto == -1 {
                    proto = 0;
                }
                core::mem::ManuallyDrop::into_inner(sock)
            } else {
                if family == -1 {
                    family = AF_INET;
                }
                if kind == -1 {
                    kind = SOCK_STREAM;
                }
                if proto == -1 {
                    proto = 0;
                }
                Socket::new(family, kind, proto).map_err(|e| e.to_pyexception(vm))?
            };
            let timeout = getdefaulttimeout();
            if let Err(err) = socket.set_nonblocking(timeout.is_some()) {
                if args.fileno.is_some() {
                    socket.into_raw_fd();
                }
                return Err(err.to_pyexception(vm));
            }
            zelf.family.store(family, Ordering::Relaxed);
            zelf.kind.store(kind, Ordering::Relaxed);
            zelf.proto.store(proto, Ordering::Relaxed);
            *zelf.timeout.lock() = timeout;
            *zelf.socket.lock() = Some(socket);
            Ok(())
        }
    }

    impl PySocket {
        fn with_socket<T>(
            &self,
            vm: &VirtualMachine,
            f: impl FnOnce(&Socket) -> io::Result<T>,
        ) -> PyResult<T> {
            let socket = self.socket.lock();
            let socket = socket
                .as_ref()
                .ok_or_else(|| io::Error::from_raw_os_error(errno::EBADF).to_pyexception(vm))?;
            f(socket).map_err(|e| e.to_pyexception(vm))
        }

        fn address(&self, object: PyObjectRef, vm: &VirtualMachine) -> PyResult<SocketAddr> {
            let tuple = PyTupleRef::try_from_object(vm, object)?;
            let fields = tuple.as_slice();
            let family = self.family.load(Ordering::Relaxed);
            if (family == AF_INET && fields.len() != 2)
                || (family == AF_INET6 && !(2..=4).contains(&fields.len()))
            {
                return Err(vm.new_type_error("invalid socket address tuple"));
            }
            if fields.len() < 2 {
                return Err(vm.new_type_error("invalid socket address tuple"));
            }
            let host = hostname(
                ArgStrOrBytesLike::try_from_object(vm, fields[0].clone())?,
                vm,
            )?;
            let port = i32::try_from_object(vm, fields[1].clone())?;
            let port =
                u16::try_from(port).map_err(|_| vm.new_overflow_error("port must be 0-65535"))?;
            let host = if host.is_empty() {
                if family == AF_INET6 { "::" } else { "0.0.0.0" }
            } else if host == "<broadcast>" && family == AF_INET {
                "255.255.255.255"
            } else {
                &host
            };
            let mut addr = addresses(host, port, family, false, vm)?.remove(0);
            if let SocketAddr::V6(ref mut addr) = addr {
                let flow = fields
                    .get(2)
                    .map(|obj| u32::try_from_object(vm, obj.clone()))
                    .transpose()?
                    .unwrap_or(0);
                let scope = fields
                    .get(3)
                    .map(|obj| u32::try_from_object(vm, obj.clone()))
                    .transpose()?
                    .unwrap_or(0);
                if flow > 0xfffff {
                    return Err(vm.new_overflow_error("flowinfo must be 0-1048575"));
                }
                // WASIX's address ABI has no flowinfo/scope fields.
                if flow != 0 || scope != 0 {
                    return Err(unsupported(vm, "IPv6 flowinfo and scope IDs"));
                }
                addr.set_flowinfo(flow);
                addr.set_scope_id(scope);
            }
            Ok(addr)
        }

        fn connect_inner(&self, socket: &Socket, address: &SocketAddr) -> io::Result<()> {
            let deadline = self.deadline();
            match socket.connect(address) {
                Err(err)
                    if deadline.is_some()
                        && matches!(
                            err.raw_os_error(),
                            Some(errno::EINPROGRESS | errno::EALREADY)
                        ) =>
                {
                    let deadline = deadline.unwrap();
                    loop {
                        let remaining = deadline.saturating_duration_since(Instant::now());
                        if remaining.is_zero() {
                            return Err(io::Error::from_raw_os_error(errno::ETIMEDOUT));
                        }
                        match socket.wait(false, true, remaining) {
                            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
                            Err(err) => return Err(err),
                            Ok(false) => {
                                return Err(io::Error::from_raw_os_error(errno::ETIMEDOUT));
                            }
                            Ok(true) => break,
                        }
                    }
                    let code = socket.get_option(SOL_SOCKET, SO_ERROR)?;
                    if code == 0 {
                        Ok(())
                    } else {
                        Err(io::Error::from_raw_os_error(code))
                    }
                }
                result => result,
            }
        }

        fn deadline(&self) -> Option<Instant> {
            self.timeout
                .lock()
                .filter(|&t| t > 0.0)
                .and_then(|t| Instant::now().checked_add(Duration::from_secs_f64(t)))
        }

        fn operation<T>(
            &self,
            write: bool,
            deadline: Option<Instant>,
            vm: &VirtualMachine,
            mut f: impl FnMut(&Socket) -> io::Result<T>,
        ) -> PyResult<T> {
            loop {
                if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                    return Err(vm.new_exception_msg(
                        vm.ctx.exceptions.timeout_error.to_owned(),
                        "timed out".into(),
                    ));
                }
                let guard = self.socket.lock();
                let socket = guard
                    .as_ref()
                    .ok_or_else(|| io::Error::from_raw_os_error(errno::EBADF).to_pyexception(vm))?;
                match f(socket) {
                    Ok(value) => return Ok(value),
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {
                        drop(guard);
                        vm.check_signals()?;
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock && deadline.is_some() => {
                        let remaining = deadline.unwrap().saturating_duration_since(Instant::now());
                        let ready = if remaining.is_zero() {
                            false
                        } else {
                            match socket.wait(!write, write, remaining) {
                                Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                                    drop(guard);
                                    vm.check_signals()?;
                                    continue;
                                }
                                result => result.map_err(|error| error.to_pyexception(vm))?,
                            }
                        };
                        if !ready {
                            return Err(vm.new_exception_msg(
                                vm.ctx.exceptions.timeout_error.to_owned(),
                                "timed out".into(),
                            ));
                        }
                    }
                    Err(e) => return Err(e.to_pyexception(vm)),
                }
            }
        }
    }

    #[pyclass(with(Constructor, Initializer), flags(BASETYPE))]
    impl PySocket {
        #[pymethod]
        fn fileno(zelf: &Py<Self>) -> i32 {
            zelf.socket.lock().as_ref().map_or(-1, Socket::as_raw_fd)
        }
        #[pymethod]
        fn close(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<()> {
            let socket = zelf.socket.lock().take();
            if let Some(socket) = socket {
                socket.close().map_err(|err| err.to_pyexception(vm))?;
            }
            Ok(())
        }
        #[pymethod]
        fn detach(zelf: &Py<Self>) -> i32 {
            zelf.socket.lock().take().map_or(-1, Socket::into_raw_fd)
        }
        #[pymethod]
        fn gettimeout(zelf: &Py<Self>) -> Option<f64> {
            *zelf.timeout.lock()
        }
        #[pymethod]
        fn settimeout(
            zelf: &Py<Self>,
            timeout: Option<ArgIntoFloat>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let timeout = parse_timeout(timeout, vm)?;
            zelf.with_socket(vm, |socket| socket.set_nonblocking(timeout.is_some()))?;
            *zelf.timeout.lock() = timeout;
            Ok(())
        }
        #[pymethod]
        fn setblocking(zelf: &Py<Self>, blocking: bool, vm: &VirtualMachine) -> PyResult<()> {
            zelf.with_socket(vm, |socket| socket.set_nonblocking(!blocking))?;
            *zelf.timeout.lock() = if blocking { None } else { Some(0.0) };
            Ok(())
        }
        #[pymethod]
        fn getblocking(zelf: &Py<Self>) -> bool {
            *zelf.timeout.lock() != Some(0.0)
        }
        #[pymethod]
        fn getsockopt(
            zelf: &Py<Self>,
            level: i32,
            optname: i32,
            buflen: OptionalArg<i32>,
            vm: &VirtualMachine,
        ) -> PyResult<PyObjectRef> {
            if buflen.unwrap_or(0) != 0 {
                return Err(unsupported(vm, "byte-valued socket options"));
            }
            zelf.with_socket(vm, |socket| socket.get_option(level, optname))
                .map(|n| n.to_pyobject(vm))
        }
        #[pymethod]
        fn setsockopt(
            zelf: &Py<Self>,
            level: i32,
            optname: i32,
            value: i32,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            zelf.with_socket(vm, |socket| socket.set_option(level, optname, value))
        }
        #[pymethod]
        fn bind(zelf: &Py<Self>, address: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            let address = zelf.address(address, vm)?;
            zelf.with_socket(vm, |socket| socket.bind(&address))
        }
        #[pymethod]
        fn connect(zelf: &Py<Self>, address: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            let address = zelf.address(address, vm)?;
            zelf.with_socket(vm, |socket| zelf.connect_inner(socket, &address))
        }
        #[pymethod]
        fn connect_ex(zelf: &Py<Self>, address: PyObjectRef, vm: &VirtualMachine) -> PyResult<i32> {
            // Parse first: name resolution and argument errors are exceptions.
            let address = zelf.address(address, vm)?;
            let socket = zelf.socket.lock();
            Ok(match socket.as_ref() {
                Some(socket) => match zelf.connect_inner(socket, &address) {
                    Ok(()) => 0,
                    Err(err) => err.raw_os_error().unwrap_or(errno::EIO),
                },
                None => errno::EBADF,
            })
        }

        #[pymethod]
        fn listen(zelf: &Py<Self>, backlog: OptionalArg<i32>, vm: &VirtualMachine) -> PyResult<()> {
            zelf.with_socket(vm, |socket| socket.listen(backlog.unwrap_or(128).max(0)))
        }
        #[pymethod]
        fn _accept(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<(i32, PyObjectRef)> {
            let (socket, address) = zelf.operation(false, zelf.deadline(), vm, Socket::accept)?;
            Ok((socket.into_raw_fd(), addr_object(address, vm)))
        }
        #[pymethod]
        fn send(
            zelf: &Py<Self>,
            data: ArgBytesLike,
            flags: OptionalArg<i32>,
            vm: &VirtualMachine,
        ) -> PyResult<usize> {
            let data = data.borrow_buf_unlocked(vm)?;
            zelf.operation(true, zelf.deadline(), vm, |socket| {
                socket.send(&data, flags.unwrap_or(0))
            })
        }
        #[pymethod]
        fn sendall(
            zelf: &Py<Self>,
            data: ArgBytesLike,
            flags: OptionalArg<i32>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let data = data.borrow_buf_unlocked(vm)?;
            let deadline = zelf.deadline();
            let mut offset = 0;
            loop {
                let n = zelf.operation(true, deadline, vm, |socket| {
                    socket.send(&data[offset..], flags.unwrap_or(0))
                })?;
                offset += n;
                if offset == data.len() {
                    return Ok(());
                }
                if n == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "socket connection broken",
                    )
                    .to_pyexception(vm));
                }
                vm.check_signals()?;
            }
        }
        #[pymethod]
        fn recv(
            zelf: &Py<Self>,
            bufsize: isize,
            flags: OptionalArg<i32>,
            vm: &VirtualMachine,
        ) -> PyResult<PyBytesRef> {
            let mut bytes = recv_buffer(bufsize, vm)?;
            if !bytes.is_empty() {
                let n = zelf.operation(false, zelf.deadline(), vm, |socket| {
                    socket.recv(&mut bytes, flags.unwrap_or(0))
                })?;
                bytes.truncate(n);
            }
            Ok(vm.ctx.new_bytes(bytes))
        }
        #[pymethod]
        fn recv_into(zelf: &Py<Self>, args: RecvIntoArgs, vm: &VirtualMachine) -> PyResult<usize> {
            let RecvIntoArgs {
                buffer,
                nbytes: n,
                flags,
            } = args;
            if n < 0 {
                return Err(vm.new_value_error("negative buffersize in recv_into"));
            }
            let n = if n == 0 { buffer.len() } else { n as usize };
            if n > buffer.len() {
                return Err(vm.new_value_error("buffer too small for requested bytes"));
            }
            if n == 0 {
                return Ok(0);
            }
            let mut bytes = recv_buffer(n as isize, vm)?;
            let read = zelf.operation(false, zelf.deadline(), vm, |socket| {
                socket.recv(&mut bytes, flags)
            })?;
            buffer.borrow_buf_mut()[..read].copy_from_slice(&bytes[..read]);
            Ok(read)
        }
        #[pymethod]
        fn sendto(
            zelf: &Py<Self>,
            data: ArgBytesLike,
            args: FuncArgs,
            vm: &VirtualMachine,
        ) -> PyResult<usize> {
            if !args.kwargs.is_empty() || !(1..=2).contains(&args.args.len()) {
                return Err(vm.new_type_error("sendto requires data, [flags,] address"));
            }
            let (flags, address) = if args.args.len() == 1 {
                (0, args.args[0].clone())
            } else {
                (
                    i32::try_from_object(vm, args.args[0].clone())?,
                    args.args[1].clone(),
                )
            };
            let address = zelf.address(address, vm)?;
            let data = data.borrow_buf_unlocked(vm)?;
            zelf.operation(true, zelf.deadline(), vm, |socket| {
                socket.send_to(&data, flags, &address)
            })
        }
        #[pymethod]
        fn recvfrom(
            zelf: &Py<Self>,
            bufsize: isize,
            flags: OptionalArg<i32>,
            vm: &VirtualMachine,
        ) -> PyResult<(PyBytesRef, PyObjectRef)> {
            let mut bytes = recv_buffer(bufsize, vm)?;
            let (n, address) = zelf.operation(false, zelf.deadline(), vm, |socket| {
                socket.recv_from(&mut bytes, flags.unwrap_or(0))
            })?;
            bytes.truncate(n);
            Ok((vm.ctx.new_bytes(bytes), addr_object(address, vm)))
        }
        #[pymethod]
        fn recvfrom_into(
            zelf: &Py<Self>,
            args: RecvIntoArgs,
            vm: &VirtualMachine,
        ) -> PyResult<(usize, PyObjectRef)> {
            let RecvIntoArgs {
                buffer,
                nbytes: n,
                flags,
            } = args;
            if n < 0 {
                return Err(vm.new_value_error("negative buffersize in recvfrom_into"));
            }
            let n = if n == 0 { buffer.len() } else { n as usize };
            if n > buffer.len() {
                return Err(vm.new_value_error("nbytes is greater than the length of the buffer"));
            }
            let mut bytes = recv_buffer(n as isize, vm)?;
            let (read, address) = zelf.operation(false, zelf.deadline(), vm, |socket| {
                socket.recv_from(&mut bytes, flags)
            })?;
            buffer.borrow_buf_mut()[..read].copy_from_slice(&bytes[..read]);
            Ok((read, addr_object(address, vm)))
        }
        #[pymethod]
        fn shutdown(zelf: &Py<Self>, how: i32, vm: &VirtualMachine) -> PyResult<()> {
            zelf.with_socket(vm, |socket| socket.shutdown(how))
        }
        #[pymethod]
        fn getsockname(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            zelf.with_socket(vm, Socket::local_addr)
                .map(|a| addr_object(a, vm))
        }
        #[pymethod]
        fn getpeername(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            zelf.with_socket(vm, Socket::peer_addr)
                .map(|a| addr_object(a, vm))
        }
    }

    fn recv_buffer(size: isize, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        let size =
            usize::try_from(size).map_err(|_| vm.new_value_error("negative buffersize in recv"))?;
        let mut buffer = Vec::new();
        buffer
            .try_reserve_exact(size)
            .map_err(|_| vm.new_memory_error("unable to allocate receive buffer"))?;
        buffer.resize(size, 0);
        Ok(buffer)
    }

    #[pyfunction]
    fn dup(fd: i32, vm: &VirtualMachine) -> PyResult<i32> {
        let socket = core::mem::ManuallyDrop::new(
            unsafe { Socket::from_raw_fd(fd) }.map_err(|e| e.to_pyexception(vm))?,
        );
        socket
            .try_clone()
            .map(Socket::into_raw_fd)
            .map_err(|e| e.to_pyexception(vm))
    }

    #[pyfunction]
    fn close(fd: i32, vm: &VirtualMachine) -> PyResult<()> {
        unsafe { Socket::from_raw_fd(fd) }
            .and_then(Socket::close)
            .map_err(|err| err.to_pyexception(vm))
    }
}
