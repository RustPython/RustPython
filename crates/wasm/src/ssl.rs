pub(crate) use _ssl::module_def;

#[allow(non_snake_case, non_upper_case_globals)]
#[pymodule]
mod _ssl {
    use rustls::pki_types::ServerName;
    use rustls::{ClientConfig, ClientConnection, Connection};
    use rustpython_host_env::ssl::{
        self as host_ssl, MemoryBio, ProtoVersion, TlsConnection, TlsError, cipher, oid,
        providers::CryptoExt, rustls_versions, validate_hostname, verify::NoVerifier,
    };
    use rustpython_vm::{
        Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
        builtins::{
            PyBaseException, PyBaseExceptionRef, PyBytesRef, PyOSError, PyStrRef, PyType,
            PyUtf8StrRef,
        },
        common::lock::{PyMutex, PyRwLock},
        function::{ArgBytesLike, OptionalArg, OptionalOption},
        stdlib::_warnings,
        types::Constructor,
    };
    use std::sync::Arc;

    #[pyattr]
    const PROTOCOL_TLS: i32 = host_ssl::PROTOCOL_TLS;
    #[pyattr]
    const PROTOCOL_SSLv23: i32 = PROTOCOL_TLS;
    #[pyattr]
    const PROTOCOL_TLS_CLIENT: i32 = host_ssl::PROTOCOL_TLS_CLIENT;
    #[pyattr]
    const PROTOCOL_TLS_SERVER: i32 = host_ssl::PROTOCOL_TLS_SERVER;
    #[pyattr]
    const PROTOCOL_TLSv1: i32 = host_ssl::PROTOCOL_TLSV1;
    #[pyattr]
    const PROTOCOL_TLSv1_1: i32 = host_ssl::PROTOCOL_TLSV1_1;
    #[pyattr]
    const PROTOCOL_TLSv1_2: i32 = host_ssl::PROTOCOL_TLSV1_2;
    #[pyattr]
    const PROTOCOL_TLSv1_3: i32 = host_ssl::PROTOCOL_TLSV1_3;

    #[pyattr]
    const PROTO_SSLv3: i32 = ProtoVersion::Ssl3 as i32;
    #[pyattr]
    const PROTO_TLSv1: i32 = ProtoVersion::Tls1 as i32;
    #[pyattr]
    const PROTO_TLSv1_1: i32 = ProtoVersion::Tls1_1 as i32;
    #[pyattr]
    const PROTO_TLSv1_2: i32 = host_ssl::PROTO_TLSV1_2;
    #[pyattr]
    const PROTO_TLSv1_3: i32 = host_ssl::PROTO_TLSV1_3;
    #[pyattr]
    const PROTO_MINIMUM_SUPPORTED: i32 = ProtoVersion::MinSupported as i32;
    #[pyattr]
    const PROTO_MAXIMUM_SUPPORTED: i32 = ProtoVersion::MaxSupported as i32;

    #[pyattr]
    const CERT_NONE: i32 = host_ssl::CERT_NONE;
    #[pyattr]
    const CERT_OPTIONAL: i32 = host_ssl::CERT_OPTIONAL;
    #[pyattr]
    const CERT_REQUIRED: i32 = host_ssl::CERT_REQUIRED;

    #[pyattr]
    const VERIFY_DEFAULT: i32 = host_ssl::VERIFY_DEFAULT;
    #[pyattr]
    const VERIFY_CRL_CHECK_LEAF: i32 = host_ssl::VERIFY_CRL_CHECK_LEAF;
    #[pyattr]
    const VERIFY_CRL_CHECK_CHAIN: i32 = host_ssl::VERIFY_CRL_CHECK_CHAIN;
    #[pyattr]
    const VERIFY_X509_STRICT: i32 = host_ssl::VERIFY_X509_STRICT;
    #[pyattr]
    const VERIFY_ALLOW_PROXY_CERTS: i32 = host_ssl::VERIFY_ALLOW_PROXY_CERTS;
    #[pyattr]
    const VERIFY_X509_TRUSTED_FIRST: i32 = host_ssl::VERIFY_X509_TRUSTED_FIRST;
    #[pyattr]
    const VERIFY_X509_PARTIAL_CHAIN: i32 = host_ssl::VERIFY_X509_PARTIAL_CHAIN;
    #[pyattr]
    const HOSTFLAG_NEVER_CHECK_SUBJECT: i32 = host_ssl::HOSTFLAG_NEVER_CHECK_SUBJECT;

    #[pyattr]
    const OP_NO_SSLv2: i32 = 0;
    #[pyattr]
    const OP_NO_SSLv3: i32 = 0x0200_0000;
    #[pyattr]
    const OP_NO_TLSv1: i32 = 0x0400_0000;
    #[pyattr]
    const OP_NO_TLSv1_1: i32 = 0x1000_0000;
    #[pyattr]
    const OP_NO_TLSv1_2: i32 = host_ssl::OP_NO_TLSV1_2;
    #[pyattr]
    const OP_NO_TLSv1_3: i32 = host_ssl::OP_NO_TLSV1_3;
    #[pyattr]
    const OP_NO_COMPRESSION: i32 = 0x0002_0000;
    #[pyattr]
    const OP_CIPHER_SERVER_PREFERENCE: i32 = 0x0040_0000;
    #[pyattr]
    const OP_SINGLE_DH_USE: i32 = 0;
    #[pyattr]
    const OP_SINGLE_ECDH_USE: i32 = 0;
    #[pyattr]
    const OP_NO_TICKET: i32 = 0x0000_4000;
    #[pyattr]
    const OP_LEGACY_SERVER_CONNECT: i32 = 0x0000_0004;
    #[pyattr]
    const OP_NO_RENEGOTIATION: i32 = 0x4000_0000;
    #[pyattr]
    const OP_IGNORE_UNEXPECTED_EOF: i32 = 0x0000_0080;
    #[pyattr]
    const OP_ENABLE_MIDDLEBOX_COMPAT: i32 = 0x0010_0000;
    #[pyattr]
    const OP_ALL: i32 = 0x0000_0BFB;

    #[pyattr]
    const ALERT_DESCRIPTION_CLOSE_NOTIFY: i32 = 0;
    #[pyattr]
    const ALERT_DESCRIPTION_UNEXPECTED_MESSAGE: i32 = 10;
    #[pyattr]
    const ALERT_DESCRIPTION_BAD_RECORD_MAC: i32 = 20;
    #[pyattr]
    const ALERT_DESCRIPTION_HANDSHAKE_FAILURE: i32 = 40;
    #[pyattr]
    const ALERT_DESCRIPTION_BAD_CERTIFICATE: i32 = 42;
    #[pyattr]
    const ALERT_DESCRIPTION_CERTIFICATE_EXPIRED: i32 = 45;
    #[pyattr]
    const ALERT_DESCRIPTION_UNKNOWN_CA: i32 = 48;
    #[pyattr]
    const ALERT_DESCRIPTION_DECODE_ERROR: i32 = 50;
    #[pyattr]
    const ALERT_DESCRIPTION_PROTOCOL_VERSION: i32 = 70;
    #[pyattr]
    const ALERT_DESCRIPTION_INTERNAL_ERROR: i32 = 80;
    #[pyattr]
    const ALERT_DESCRIPTION_UNRECOGNIZED_NAME: i32 = 112;

    #[pyattr]
    const SSL_ERROR_NONE: i32 = 0;
    #[pyattr]
    const SSL_ERROR_SSL: i32 = 1;
    #[pyattr]
    const SSL_ERROR_WANT_READ: i32 = 2;
    #[pyattr]
    const SSL_ERROR_WANT_WRITE: i32 = 3;
    #[pyattr]
    const SSL_ERROR_WANT_X509_LOOKUP: i32 = 4;
    #[pyattr]
    const SSL_ERROR_SYSCALL: i32 = 5;
    #[pyattr]
    const SSL_ERROR_ZERO_RETURN: i32 = 6;
    #[pyattr]
    const SSL_ERROR_WANT_CONNECT: i32 = 7;
    #[pyattr]
    const SSL_ERROR_EOF: i32 = 8;
    #[pyattr]
    const SSL_ERROR_INVALID_ERROR_CODE: i32 = 10;

    #[pyattr]
    const OPENSSL_VERSION_NUMBER: i32 = 0x3030_0000;
    #[pyattr]
    const OPENSSL_VERSION: &str = "OpenSSL 3.3.0-compatible (rustls-rustcrypto)";
    #[pyattr]
    const OPENSSL_VERSION_INFO: (i32, i32, i32, i32, i32) = (3, 3, 0, 0, 15);
    #[pyattr]
    const _OPENSSL_API_VERSION: (i32, i32, i32, i32, i32) = (3, 3, 0, 0, 15);

    #[pyattr(once)]
    fn _DEFAULT_CIPHERS(_vm: &VirtualMachine) -> String {
        cipher::default_cipher_string()
    }

    #[pyattr]
    const HAS_SNI: bool = true;
    #[pyattr]
    const HAS_TLS_UNIQUE: bool = false;
    #[pyattr]
    const HAS_ECDH: bool = true;
    #[pyattr]
    const HAS_NPN: bool = false;
    #[pyattr]
    const HAS_ALPN: bool = true;
    #[pyattr]
    const HAS_PSK: bool = false;
    #[pyattr]
    const HAS_SSLv2: bool = false;
    #[pyattr]
    const HAS_SSLv3: bool = false;
    #[pyattr]
    const HAS_TLSv1: bool = false;
    #[pyattr]
    const HAS_TLSv1_1: bool = false;
    #[pyattr]
    const HAS_TLSv1_2: bool = true;
    #[pyattr]
    const HAS_TLSv1_3: bool = true;
    #[pyattr]
    const HAS_PHA: bool = false;

    #[pyattr]
    const ENCODING_PEM: i32 = 1;
    #[pyattr]
    const ENCODING_DER: i32 = 2;
    #[pyattr]
    const ENCODING_PEM_AUX: i32 = 0x101;

    #[pyattr]
    #[pyexception(name = "SSLError", base = PyOSError)]
    #[derive(Debug)]
    #[repr(transparent)]
    struct PySSLError(PyOSError);

    #[pyexception]
    impl PySSLError {
        #[pymethod]
        fn __str__(zelf: &Py<PyBaseException>, vm: &VirtualMachine) -> PyResult<PyStrRef> {
            use rustpython_vm::AsObject;
            if let Ok(strerror) = zelf.as_object().get_attr("strerror", vm)
                && !vm.is_none(&strerror)
            {
                return strerror.str(vm);
            }
            let args = zelf.args();
            if args.len() == 1 {
                args.as_slice()[0].str(vm)
            } else {
                args.as_object().str(vm)
            }
        }
    }

    #[pyattr]
    #[pyexception(name = "SSLZeroReturnError", base = PySSLError, impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    struct PySSLZeroReturnError(PySSLError);

    #[pyattr]
    #[pyexception(name = "SSLWantReadError", base = PySSLError, impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    struct PySSLWantReadError(PySSLError);

    #[pyattr]
    #[pyexception(name = "SSLWantWriteError", base = PySSLError, impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    struct PySSLWantWriteError(PySSLError);

    #[pyattr]
    #[pyexception(name = "SSLSyscallError", base = PySSLError, impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    struct PySSLSyscallError(PySSLError);

    #[pyattr]
    #[pyexception(name = "SSLEOFError", base = PySSLError, impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    struct PySSLEOFError(PySSLError);

    #[pyattr]
    #[pyexception(name = "SSLCertVerificationError", base = PySSLError, impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    struct PySSLCertVerificationError(PySSLError);

    fn ssl_error(vm: &VirtualMachine, msg: impl Into<String>) -> rustpython_vm::PyRef<PyOSError> {
        vm.new_os_subtype_error(PySSLError::class(&vm.ctx).to_owned(), None, msg.into())
    }

    fn map_tls(vm: &VirtualMachine, err: TlsError) -> PyBaseExceptionRef {
        match err {
            TlsError::WantRead => vm
                .new_os_subtype_error(
                    PySSLWantReadError::class(&vm.ctx).to_owned(),
                    Some(SSL_ERROR_WANT_READ),
                    "The operation did not complete (read)",
                )
                .upcast(),
            TlsError::WantWrite => vm
                .new_os_subtype_error(
                    PySSLWantWriteError::class(&vm.ctx).to_owned(),
                    Some(SSL_ERROR_WANT_WRITE),
                    "The operation did not complete (write)",
                )
                .upcast(),
            TlsError::ZeroReturn => vm
                .new_os_subtype_error(
                    PySSLZeroReturnError::class(&vm.ctx).to_owned(),
                    Some(SSL_ERROR_ZERO_RETURN),
                    "TLS/SSL connection has been closed (EOF)",
                )
                .upcast(),
            TlsError::Eof => vm
                .new_os_subtype_error(
                    PySSLEOFError::class(&vm.ctx).to_owned(),
                    Some(SSL_ERROR_EOF),
                    "EOF occurred in violation of protocol",
                )
                .upcast(),
            TlsError::CertVerification(err) => vm
                .new_os_subtype_error(
                    PySSLCertVerificationError::class(&vm.ctx).to_owned(),
                    Some(SSL_ERROR_SSL),
                    format!("{err}"),
                )
                .upcast(),
            other => ssl_error(vm, format!("{other:?}")).upcast(),
        }
    }

    #[pyattr]
    #[pyclass(name = "MemoryBIO", module = "ssl")]
    #[derive(Debug, PyPayload)]
    struct PyMemoryBIO {
        inner: PyMutex<MemoryBio>,
    }

    #[pyclass(with(Constructor), flags(BASETYPE))]
    impl PyMemoryBIO {
        #[pymethod]
        fn read(&self, len: OptionalArg<i32>, vm: &VirtualMachine) -> PyResult<PyBytesRef> {
            let mut bio = self.inner.lock();
            let read_len = match len {
                OptionalArg::Present(n) if n >= 0 => n as usize,
                OptionalArg::Present(n) => {
                    return Err(vm.new_value_error(format!("negative read length: {n}")));
                }
                OptionalArg::Missing => bio.pending(),
            };
            Ok(vm.ctx.new_bytes(bio.read(read_len)))
        }

        #[pymethod]
        fn write(&self, buf: ArgBytesLike, vm: &VirtualMachine) -> PyResult<usize> {
            self.inner
                .lock()
                .write(&buf.borrow_buf())
                .map_err(|err| ssl_error(vm, err.to_string()).upcast())
        }

        #[pymethod]
        fn write_eof(&self) {
            self.inner.lock().write_eof();
        }

        #[pygetset]
        fn pending(&self) -> i32 {
            self.inner.lock().pending() as i32
        }

        #[pygetset]
        fn eof(&self) -> bool {
            self.inner.lock().eof()
        }
    }

    impl Constructor for PyMemoryBIO {
        type Args = ();

        fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
            Ok(Self {
                inner: PyMutex::new(MemoryBio::new()),
            })
        }
    }

    #[pyattr]
    #[pyclass(name = "SSLSession", module = "ssl")]
    #[derive(Debug, PyPayload)]
    struct PySSLSession {}

    #[pyclass(flags(BASETYPE))]
    impl PySSLSession {
        #[pygetset]
        fn id(&self) -> Vec<u8> {
            Vec::new()
        }

        #[pygetset]
        fn time(&self) -> i32 {
            0
        }

        #[pygetset]
        fn timeout(&self) -> i32 {
            0
        }

        #[pygetset]
        fn ticket_lifetime_hint(&self) -> i32 {
            0
        }

        #[pygetset]
        fn has_ticket(&self) -> bool {
            false
        }
    }

    #[pyattr]
    #[pyclass(name = "_SSLContext", module = "ssl")]
    #[derive(Debug, PyPayload)]
    struct PySSLContext {
        protocol: i32,
        check_hostname: PyRwLock<bool>,
        host_flags: PyRwLock<i32>,
        verify_mode: PyRwLock<i32>,
        verify_flags: PyRwLock<i32>,
        options: PyRwLock<i32>,
        minimum_version: PyRwLock<i32>,
        maximum_version: PyRwLock<i32>,
        alpn_protocols: PyRwLock<Vec<Vec<u8>>>,
    }

    #[derive(FromArgs)]
    struct WrapBioArgs {
        incoming: PyRef<PyMemoryBIO>,
        outgoing: PyRef<PyMemoryBIO>,
        #[pyarg(named, optional)]
        server_side: OptionalArg<bool>,
        #[pyarg(named, optional)]
        server_hostname: OptionalArg<Option<PyUtf8StrRef>>,
        #[pyarg(named, optional)]
        owner: OptionalArg<PyObjectRef>,
        #[pyarg(named, optional)]
        session: OptionalArg<PyObjectRef>,
    }

    #[derive(FromArgs)]
    struct WrapSocketArgs {
        sock: PyObjectRef,
        server_side: bool,
        #[pyarg(positional, optional)]
        server_hostname: OptionalArg<Option<PyUtf8StrRef>>,
        #[pyarg(named, optional)]
        owner: OptionalArg<PyObjectRef>,
        #[pyarg(named, optional)]
        session: OptionalArg<PyObjectRef>,
    }

    #[pyclass(with(Constructor), flags(BASETYPE))]
    impl PySSLContext {
        #[pygetset]
        fn check_hostname(&self) -> bool {
            *self.check_hostname.read()
        }

        #[pygetset(setter)]
        fn set_check_hostname(&self, value: bool) {
            *self.check_hostname.write() = value;
        }

        #[pygetset]
        fn verify_mode(&self) -> i32 {
            *self.verify_mode.read()
        }

        #[pygetset(setter)]
        fn set_verify_mode(&self, mode: i32, vm: &VirtualMachine) -> PyResult<()> {
            if !(CERT_NONE..=CERT_REQUIRED).contains(&mode) {
                return Err(vm.new_value_error(format!("invalid value for verify_mode")));
            }
            if mode == CERT_NONE && *self.check_hostname.read() {
                return Err(vm.new_value_error(
                    "Cannot set verify_mode to CERT_NONE when check_hostname is enabled",
                ));
            }
            *self.verify_mode.write() = mode;
            Ok(())
        }

        #[pygetset]
        fn verify_flags(&self) -> i32 {
            *self.verify_flags.read()
        }

        #[pygetset(setter)]
        fn set_verify_flags(&self, flags: i32) {
            *self.verify_flags.write() = flags;
        }

        #[pygetset]
        fn options(&self) -> i32 {
            *self.options.read()
        }

        #[pygetset(setter)]
        fn set_options(&self, options: i32) {
            *self.options.write() = options;
        }

        #[pygetset]
        fn minimum_version(&self) -> i32 {
            *self.minimum_version.read()
        }

        #[pygetset(setter)]
        fn set_minimum_version(&self, value: i32) {
            *self.minimum_version.write() = value;
        }

        #[pygetset]
        fn maximum_version(&self) -> i32 {
            *self.maximum_version.read()
        }

        #[pygetset(setter)]
        fn set_maximum_version(&self, value: i32) {
            *self.maximum_version.write() = value;
        }

        #[pygetset]
        fn _host_flags(&self) -> i32 {
            *self.host_flags.read()
        }

        #[pygetset(setter)]
        fn set__host_flags(&self, flags: i32) {
            *self.host_flags.write() = flags;
        }

        #[pygetset]
        fn protocol(&self) -> i32 {
            self.protocol
        }

        #[pygetset]
        fn security_level(&self) -> i32 {
            2
        }

        #[pymethod]
        fn set_ciphers(&self, _ciphers: PyUtf8StrRef) {}

        #[pymethod]
        fn _set_alpn_protocols(&self, protos: ArgBytesLike, vm: &VirtualMachine) -> PyResult<()> {
            let parsed = rustpython_host_env::ssl::parse_length_prefixed_alpn(&protos.borrow_buf())
                .map_err(|err| vm.new_value_error(err.0))?;
            *self.alpn_protocols.write() = parsed;
            Ok(())
        }

        #[pymethod]
        fn set_default_verify_paths(&self) {}

        #[pymethod]
        fn load_verify_locations(
            &self,
            _cafile: OptionalOption<PyObjectRef>,
            _capath: OptionalOption<PyObjectRef>,
            _cadata: OptionalOption<PyObjectRef>,
        ) {
        }

        #[pymethod]
        fn load_cert_chain(
            &self,
            _certfile: PyObjectRef,
            _keyfile: OptionalOption<PyObjectRef>,
            _password: OptionalOption<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            Err(ssl_error(vm, "certificate files are unavailable").upcast())
        }

        #[pymethod]
        fn load_dh_params(&self, _path: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            Err(ssl_error(vm, "DH parameters are unavailable").upcast())
        }

        #[pymethod]
        fn set_ecdh_curve(&self, _name: PyObjectRef) {}

        #[pymethod]
        fn cert_store_stats(&self, vm: &VirtualMachine) -> PyObjectRef {
            vm.ctx.new_dict().into()
        }

        #[pymethod]
        fn get_ca_certs(&self, _binary_form: OptionalArg<bool>, vm: &VirtualMachine) -> PyResult {
            Ok(vm.ctx.new_list(Vec::new()).into())
        }

        #[pymethod]
        fn _wrap_bio(
            zelf: PyRef<Self>,
            args: WrapBioArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<PySSLSocket>> {
            let server_side = args.server_side.unwrap_or(false);
            let hostname = match args.server_hostname.into_option().flatten() {
                Some(name) => {
                    let hostname = name.as_str();
                    validate_hostname(hostname)
                        .map_err(|err| vm.new_value_error(err.message().to_owned()))?;
                    Some(hostname.to_owned())
                }
                None => None,
            };
            if server_side && zelf.protocol == PROTOCOL_TLS_CLIENT {
                return Err(ssl_error(
                    vm,
                    "Cannot create a server socket with a PROTOCOL_TLS_CLIENT context",
                )
                .upcast());
            }
            if !server_side && zelf.protocol == PROTOCOL_TLS_SERVER {
                return Err(ssl_error(
                    vm,
                    "Cannot create a client socket with a PROTOCOL_TLS_SERVER context",
                )
                .upcast());
            }
            let _ = (args.owner, args.session);
            PySSLSocket {
                incoming: args.incoming,
                outgoing: args.outgoing,
                context: PyRwLock::new(zelf),
                server_side,
                server_hostname: hostname,
                connection: PyMutex::new(None),
            }
            .into_ref_with_type(vm, vm.class("_ssl", "_SSLSocket"))
        }

        #[pymethod]
        fn _wrap_socket(
            zelf: PyRef<Self>,
            args: WrapSocketArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<PySSLSocket>> {
            let _ = args.sock;
            Self::_wrap_bio(
                zelf,
                WrapBioArgs {
                    incoming: PyMemoryBIO {
                        inner: PyMutex::new(MemoryBio::new()),
                    }
                    .into_ref(&vm.ctx),
                    outgoing: PyMemoryBIO {
                        inner: PyMutex::new(MemoryBio::new()),
                    }
                    .into_ref(&vm.ctx),
                    server_side: OptionalArg::Present(args.server_side),
                    server_hostname: args.server_hostname,
                    owner: args.owner,
                    session: args.session,
                },
                vm,
            )
        }
    }

    impl Constructor for PySSLContext {
        type Args = (i32,);

        fn py_new(
            _cls: &Py<PyType>,
            (protocol,): Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult<Self> {
            let deprecated = match protocol {
                PROTOCOL_TLS => Some("PROTOCOL_TLS"),
                PROTOCOL_TLSv1_2 => Some("PROTOCOL_TLSv1_2"),
                PROTOCOL_TLS_CLIENT | PROTOCOL_TLS_SERVER | PROTOCOL_TLSv1_3 => None,
                PROTOCOL_TLSv1 | PROTOCOL_TLSv1_1 => {
                    return Err(vm.new_value_error(
                        "TLS 1.0 and 1.1 are not supported by rustls for security reasons",
                    ));
                }
                _ => {
                    return Err(vm.new_value_error(format!("invalid protocol version: {protocol}")));
                }
            };
            if let Some(name) = deprecated {
                _warnings::warn(
                    vm.ctx.exceptions.deprecation_warning,
                    format!("ssl.{name} is deprecated"),
                    2,
                    vm,
                )?;
            }

            let default_options = OP_ALL
                | OP_NO_SSLv2
                | OP_NO_SSLv3
                | OP_NO_COMPRESSION
                | OP_CIPHER_SERVER_PREFERENCE
                | OP_SINGLE_DH_USE
                | OP_SINGLE_ECDH_USE
                | OP_ENABLE_MIDDLEBOX_COMPAT;
            let verify_mode = if protocol == PROTOCOL_TLS_CLIENT {
                CERT_REQUIRED
            } else {
                CERT_NONE
            };
            let (minimum_version, maximum_version) = match protocol {
                PROTOCOL_TLSv1_2 => (PROTO_TLSv1_2, PROTO_TLSv1_2),
                PROTOCOL_TLSv1_3 => (PROTO_TLSv1_3, PROTO_TLSv1_3),
                _ => (PROTO_MINIMUM_SUPPORTED, PROTO_MAXIMUM_SUPPORTED),
            };

            Ok(Self {
                protocol,
                check_hostname: PyRwLock::new(protocol == PROTOCOL_TLS_CLIENT),
                host_flags: PyRwLock::new(0),
                verify_mode: PyRwLock::new(verify_mode),
                verify_flags: PyRwLock::new(VERIFY_DEFAULT | VERIFY_X509_TRUSTED_FIRST),
                options: PyRwLock::new(default_options),
                minimum_version: PyRwLock::new(minimum_version),
                maximum_version: PyRwLock::new(maximum_version),
                alpn_protocols: PyRwLock::new(Vec::new()),
            })
        }
    }

    #[pyattr]
    #[pyclass(name = "_SSLSocket", module = "ssl")]
    #[derive(Debug, PyPayload)]
    struct PySSLSocket {
        incoming: PyRef<PyMemoryBIO>,
        outgoing: PyRef<PyMemoryBIO>,
        context: PyRwLock<PyRef<PySSLContext>>,
        server_side: bool,
        server_hostname: Option<String>,
        connection: PyMutex<Option<TlsConnection>>,
    }

    #[pyclass]
    impl PySSLSocket {
        fn ensure_conn(&self, vm: &VirtualMachine) -> PyResult<()> {
            if self.connection.lock().is_some() {
                return Ok(());
            }
            if self.server_side {
                return Err(ssl_error(vm, "server certificates are unavailable").upcast());
            }
            let context = self.context.read().clone();
            let versions = rustls_versions(
                *context.minimum_version.read(),
                *context.maximum_version.read(),
                *context.options.read(),
            );
            let mut builder =
                ClientConfig::builder_with_provider(Arc::new(CryptoExt::get_provider().clone()))
                    .with_protocol_versions(versions)
                    .map_err(|err| ssl_error(vm, err.to_string()).upcast())?
                    .dangerous()
                    .with_custom_certificate_verifier(Arc::new(NoVerifier))
                    .with_no_client_auth();
            let alpn = context.alpn_protocols.read().clone();
            if !alpn.is_empty() {
                builder.alpn_protocols = alpn;
            }
            let hostname = self.server_hostname.as_deref().unwrap_or("localhost");
            let server_name = ServerName::try_from(hostname.to_owned())
                .map_err(|_| vm.new_value_error(format!("invalid server hostname: {hostname}")))?;
            let conn = ClientConnection::new(Arc::new(builder), server_name)
                .map_err(|err| ssl_error(vm, err.to_string()).upcast())?;
            *self.connection.lock() = Some(TlsConnection::new(Connection::Client(conn)));
            Ok(())
        }

        fn pump(&self, vm: &VirtualMachine) -> PyResult<()> {
            self.ensure_conn(vm)?;
            let mut connection = self.connection.lock();
            let conn = connection.as_mut().expect("connection created above");
            let incoming = self.incoming.inner.lock().read(usize::MAX);
            if !incoming.is_empty() {
                conn.feed_tls(&incoming).map_err(|err| map_tls(vm, err))?;
            }
            conn.process_packets().map_err(|err| map_tls(vm, err))?;
            let outgoing = conn.drain_tls().map_err(|err| map_tls(vm, err))?;
            if !outgoing.is_empty() {
                self.outgoing
                    .inner
                    .lock()
                    .write(&outgoing)
                    .map_err(|err| ssl_error(vm, err.to_string()).upcast())?;
            }
            Ok(())
        }

        #[pymethod]
        fn do_handshake(&self, vm: &VirtualMachine) -> PyResult<()> {
            self.pump(vm)?;
            let connected = self
                .connection
                .lock()
                .as_ref()
                .is_some_and(|conn| !conn.is_handshaking());
            if connected {
                Ok(())
            } else {
                Err(map_tls(vm, TlsError::WantRead))
            }
        }

        #[pymethod]
        fn read(&self, len: OptionalArg<i32>, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            let size = len.unwrap_or(1024);
            if size < 0 {
                return Err(vm.new_value_error("size should not be negative"));
            }
            self.pump(vm)?;
            let mut connection = self.connection.lock();
            let conn = connection
                .as_mut()
                .ok_or_else(|| ssl_error(vm, "SSL connection is closed").upcast())?;
            let mut buf = vec![0; size as usize];
            match std::io::Read::read(&mut conn.inner_mut().reader(), &mut buf) {
                Ok(0) => Err(map_tls(vm, TlsError::ZeroReturn)),
                Ok(n) => {
                    buf.truncate(n);
                    Ok(buf)
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    Err(map_tls(vm, TlsError::WantRead))
                }
                Err(err) => Err(ssl_error(vm, err.to_string()).upcast()),
            }
        }

        #[pymethod]
        fn write(&self, data: ArgBytesLike, vm: &VirtualMachine) -> PyResult<usize> {
            self.ensure_conn(vm)?;
            let bytes = data.borrow_buf();
            let mut connection = self.connection.lock();
            let conn = connection
                .as_mut()
                .ok_or_else(|| ssl_error(vm, "SSL connection is closed").upcast())?;
            let n = std::io::Write::write(&mut conn.inner_mut().writer(), &bytes)
                .map_err(|err| ssl_error(vm, err.to_string()).upcast())?;
            drop(connection);
            self.pump(vm)?;
            Ok(n)
        }

        #[pymethod]
        fn pending(&self) -> usize {
            self.connection
                .lock()
                .as_mut()
                .map_or(0, TlsConnection::pending_plaintext)
        }

        #[pymethod]
        fn shutdown(&self, vm: &VirtualMachine) -> PyResult<()> {
            if let Some(conn) = self.connection.lock().as_mut() {
                conn.send_close_notify();
            }
            self.pump(vm)
        }

        #[pygetset]
        fn context(&self) -> PyRef<PySSLContext> {
            self.context.read().clone()
        }

        #[pygetset(setter)]
        fn set_context(&self, ctx: PyRef<PySSLContext>) {
            *self.context.write() = ctx;
        }

        #[pygetset]
        fn server_hostname(&self) -> Option<String> {
            self.server_hostname.clone()
        }

        #[pygetset]
        fn server_side(&self) -> bool {
            self.server_side
        }

        #[pygetset]
        fn session(&self, vm: &VirtualMachine) -> PyObjectRef {
            vm.ctx.none()
        }

        #[pygetset(setter)]
        fn set_session(&self, _session: PyObjectRef) {}

        #[pygetset]
        fn session_reused(&self) -> bool {
            false
        }

        #[pymethod]
        fn peer_certificate(&self, _binary: OptionalArg<bool>, vm: &VirtualMachine) -> PyObjectRef {
            vm.ctx.none()
        }

        #[pymethod]
        fn cipher(&self, vm: &VirtualMachine) -> PyObjectRef {
            vm.ctx.none()
        }

        #[pymethod]
        fn shared_ciphers(&self, vm: &VirtualMachine) -> PyObjectRef {
            vm.ctx.none()
        }

        #[pymethod]
        fn compression(&self, vm: &VirtualMachine) -> PyObjectRef {
            vm.ctx.none()
        }

        #[pymethod]
        fn version(&self) -> Option<&'static str> {
            match self
                .connection
                .lock()
                .as_ref()
                .and_then(TlsConnection::protocol_version)
            {
                Some(rustls::ProtocolVersion::TLSv1_3) => Some("TLSv1.3"),
                Some(rustls::ProtocolVersion::TLSv1_2) => Some("TLSv1.2"),
                _ => None,
            }
        }

        #[pymethod]
        fn selected_alpn_protocol(&self) -> Option<String> {
            self.connection.lock().as_ref().and_then(|conn| {
                conn.alpn_protocol()
                    .map(|proto| String::from_utf8_lossy(proto).into_owned())
            })
        }
    }

    #[derive(FromArgs)]
    struct Txt2ObjArgs {
        txt: PyUtf8StrRef,
        #[pyarg(named, default = false)]
        name: bool,
    }

    fn oid_tuple(entry: &oid::OidEntry, vm: &VirtualMachine) -> PyObjectRef {
        vm.new_tuple((
            vm.ctx.new_int(entry.nid),
            vm.ctx.new_str(entry.short_name),
            vm.ctx.new_str(entry.long_name),
            entry
                .oid_string()
                .map_or_else(|| vm.ctx.none(), |oid| vm.ctx.new_str(oid).into()),
        ))
        .into()
    }

    #[pyfunction]
    fn txt2obj(args: Txt2ObjArgs, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let txt = args.txt.as_str();
        let entry = if txt.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            oid::find_by_oid_string(txt)
        } else if args.name {
            oid::find_by_name(txt)
        } else {
            None
        }
        .ok_or_else(|| vm.new_value_error(format!("unknown object '{txt}'")))?;
        Ok(oid_tuple(entry, vm))
    }

    #[pyfunction]
    fn nid2obj(nid: i32, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let entry = oid::find_by_nid(nid)
            .ok_or_else(|| vm.new_value_error(format!("unknown NID {nid}")))?;
        Ok(oid_tuple(entry, vm))
    }

    #[pyfunction]
    fn get_default_verify_paths(vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx
            .new_tuple(vec![
                vm.ctx.new_str("SSL_CERT_FILE").into(),
                vm.ctx.new_str("/etc/ssl/cert.pem").into(),
                vm.ctx.new_str("SSL_CERT_DIR").into(),
                vm.ctx.new_str("/etc/ssl/certs").into(),
            ])
            .into()
    }

    #[pyfunction]
    fn RAND_status() -> i32 {
        1
    }

    #[pyfunction]
    fn RAND_add(_string: PyObjectRef, _entropy: f64) {}

    #[pyfunction]
    fn RAND_bytes(n: i32, vm: &VirtualMachine) -> PyResult<PyBytesRef> {
        if n < 0 {
            return Err(vm.new_value_error("num must be positive"));
        }
        let mut buf = vm.new_zeroed_bytes(n as usize)?;
        CryptoExt::get_provider()
            .secure_random
            .fill(&mut buf)
            .map_err(|_| vm.new_os_error("Failed to generate random bytes"))?;
        Ok(PyBytesRef::from(vm.ctx.new_bytes(buf)))
    }
}
