//! Wasm `_ssl` on the rustls-free `rustpython_host_env::ssl` surface.
//!
//! MemoryBIO, protocol constants, OID lookup, hostname checks, and ALPN
//! parsing work. TLS connections are not available until a wasm TLS engine
//! is wired to the same types.

#[path = "ssl/error.rs"]
mod error;

pub(crate) use _ssl::module_def;

#[allow(non_snake_case)]
#[allow(non_upper_case_globals)]
#[pymodule(with(error::ssl_error))]
mod _ssl {
    use super::error::{PySSLError, SSL_ERROR_SSL};
    use crate::{
        common::lock::{PyMutex, PyRwLock},
        vm::{
            AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject,
            VirtualMachine,
            builtins::{PyBytesRef, PyType, PyUtf8StrRef},
            function::{ArgBytesLike, FuncArgs, OptionalArg, PyComparisonValue, PySetterValue},
            stdlib::_warnings,
            types::{Comparable, Constructor, PyComparisonOp, Representable},
        },
    };
    use rustpython_host_env::ssl::oid;
    use rustpython_vm::exceptions;

    const UNAVAILABLE: &str = "TLS is not available on this platform";

    #[pyattr]
    const PROTOCOL_TLS: i32 = rustpython_host_env::ssl::PROTOCOL_TLS;
    #[pyattr]
    const PROTOCOL_SSLv23: i32 = PROTOCOL_TLS;
    #[pyattr]
    const PROTOCOL_TLS_CLIENT: i32 = rustpython_host_env::ssl::PROTOCOL_TLS_CLIENT;
    #[pyattr]
    const PROTOCOL_TLS_SERVER: i32 = rustpython_host_env::ssl::PROTOCOL_TLS_SERVER;
    #[pyattr]
    const PROTOCOL_TLSv1: i32 = rustpython_host_env::ssl::PROTOCOL_TLSV1;
    #[pyattr]
    const PROTOCOL_TLSv1_1: i32 = rustpython_host_env::ssl::PROTOCOL_TLSV1_1;
    #[pyattr]
    const PROTOCOL_TLSv1_2: i32 = rustpython_host_env::ssl::PROTOCOL_TLSV1_2;
    #[pyattr]
    const PROTOCOL_TLSv1_3: i32 = rustpython_host_env::ssl::PROTOCOL_TLSV1_3;

    #[pyattr]
    const PROTO_SSLv3: i32 = 0x0300;
    #[pyattr]
    const PROTO_TLSv1: i32 = 0x0301;
    #[pyattr]
    const PROTO_TLSv1_1: i32 = 0x0302;
    #[pyattr]
    const PROTO_TLSv1_2: i32 = rustpython_host_env::ssl::PROTO_TLSV1_2;
    #[pyattr]
    const PROTO_TLSv1_3: i32 = rustpython_host_env::ssl::PROTO_TLSV1_3;
    #[pyattr]
    const PROTO_MINIMUM_SUPPORTED: i32 = -2;
    #[pyattr]
    const PROTO_MAXIMUM_SUPPORTED: i32 = -1;

    #[pyattr]
    const CERT_NONE: i32 = rustpython_host_env::ssl::CERT_NONE;
    #[pyattr]
    const CERT_OPTIONAL: i32 = rustpython_host_env::ssl::CERT_OPTIONAL;
    #[pyattr]
    const CERT_REQUIRED: i32 = rustpython_host_env::ssl::CERT_REQUIRED;

    #[pyattr]
    const VERIFY_DEFAULT: i32 = rustpython_host_env::ssl::VERIFY_DEFAULT;
    #[pyattr]
    const VERIFY_CRL_CHECK_LEAF: i32 = rustpython_host_env::ssl::VERIFY_CRL_CHECK_LEAF;
    #[pyattr]
    const VERIFY_CRL_CHECK_CHAIN: i32 = rustpython_host_env::ssl::VERIFY_CRL_CHECK_CHAIN;
    #[pyattr]
    const VERIFY_X509_STRICT: i32 = rustpython_host_env::ssl::VERIFY_X509_STRICT;
    #[pyattr]
    const VERIFY_ALLOW_PROXY_CERTS: i32 = rustpython_host_env::ssl::VERIFY_ALLOW_PROXY_CERTS;
    #[pyattr]
    const VERIFY_X509_TRUSTED_FIRST: i32 = rustpython_host_env::ssl::VERIFY_X509_TRUSTED_FIRST;
    #[pyattr]
    const VERIFY_X509_PARTIAL_CHAIN: i32 = rustpython_host_env::ssl::VERIFY_X509_PARTIAL_CHAIN;
    #[pyattr]
    const HOSTFLAG_NEVER_CHECK_SUBJECT: i32 =
        rustpython_host_env::ssl::HOSTFLAG_NEVER_CHECK_SUBJECT;

    #[pyattr]
    const OP_NO_SSLv2: i32 = 0x00000000;
    #[pyattr]
    const OP_NO_SSLv3: i32 = 0x02000000;
    #[pyattr]
    const OP_NO_TLSv1: i32 = 0x04000000;
    #[pyattr]
    const OP_NO_TLSv1_1: i32 = 0x10000000;
    #[pyattr]
    const OP_NO_TLSv1_2: i32 = rustpython_host_env::ssl::OP_NO_TLSV1_2;
    #[pyattr]
    const OP_NO_TLSv1_3: i32 = rustpython_host_env::ssl::OP_NO_TLSV1_3;
    #[pyattr]
    const OP_NO_COMPRESSION: i32 = 0x00020000;
    #[pyattr]
    const OP_CIPHER_SERVER_PREFERENCE: i32 = 0x00400000;
    #[pyattr]
    const OP_SINGLE_DH_USE: i32 = 0x00000000;
    #[pyattr]
    const OP_SINGLE_ECDH_USE: i32 = 0x00000000;
    #[pyattr]
    const OP_NO_TICKET: i32 = 0x00004000;
    #[pyattr]
    const OP_LEGACY_SERVER_CONNECT: i32 = 0x00000004;
    #[pyattr]
    const OP_NO_RENEGOTIATION: i32 = 0x40000000;
    #[pyattr]
    const OP_IGNORE_UNEXPECTED_EOF: i32 = 0x00000080;
    #[pyattr]
    const OP_ENABLE_MIDDLEBOX_COMPAT: i32 = 0x00100000;
    #[pyattr]
    const OP_ALL: i32 = 0x00000BFB;

    #[pyattr]
    const ALERT_DESCRIPTION_CLOSE_NOTIFY: i32 = 0;
    #[pyattr]
    const ALERT_DESCRIPTION_UNEXPECTED_MESSAGE: i32 = 10;
    #[pyattr]
    const ALERT_DESCRIPTION_BAD_RECORD_MAC: i32 = 20;
    #[pyattr]
    const ALERT_DESCRIPTION_DECRYPTION_FAILED: i32 = 21;
    #[pyattr]
    const ALERT_DESCRIPTION_RECORD_OVERFLOW: i32 = 22;
    #[pyattr]
    const ALERT_DESCRIPTION_DECOMPRESSION_FAILURE: i32 = 30;
    #[pyattr]
    const ALERT_DESCRIPTION_HANDSHAKE_FAILURE: i32 = 40;
    #[pyattr]
    const ALERT_DESCRIPTION_NO_CERTIFICATE: i32 = 41;
    #[pyattr]
    const ALERT_DESCRIPTION_BAD_CERTIFICATE: i32 = 42;
    #[pyattr]
    const ALERT_DESCRIPTION_UNSUPPORTED_CERTIFICATE: i32 = 43;
    #[pyattr]
    const ALERT_DESCRIPTION_CERTIFICATE_REVOKED: i32 = 44;
    #[pyattr]
    const ALERT_DESCRIPTION_CERTIFICATE_EXPIRED: i32 = 45;
    #[pyattr]
    const ALERT_DESCRIPTION_CERTIFICATE_UNKNOWN: i32 = 46;
    #[pyattr]
    const ALERT_DESCRIPTION_ILLEGAL_PARAMETER: i32 = 47;
    #[pyattr]
    const ALERT_DESCRIPTION_UNKNOWN_CA: i32 = 48;
    #[pyattr]
    const ALERT_DESCRIPTION_ACCESS_DENIED: i32 = 49;
    #[pyattr]
    const ALERT_DESCRIPTION_DECODE_ERROR: i32 = 50;
    #[pyattr]
    const ALERT_DESCRIPTION_DECRYPT_ERROR: i32 = 51;
    #[pyattr]
    const ALERT_DESCRIPTION_EXPORT_RESTRICTION: i32 = 60;
    #[pyattr]
    const ALERT_DESCRIPTION_PROTOCOL_VERSION: i32 = 70;
    #[pyattr]
    const ALERT_DESCRIPTION_INSUFFICIENT_SECURITY: i32 = 71;
    #[pyattr]
    const ALERT_DESCRIPTION_INTERNAL_ERROR: i32 = 80;
    #[pyattr]
    const ALERT_DESCRIPTION_INAPPROPRIATE_FALLBACK: i32 = 86;
    #[pyattr]
    const ALERT_DESCRIPTION_USER_CANCELLED: i32 = 90;
    #[pyattr]
    const ALERT_DESCRIPTION_NO_RENEGOTIATION: i32 = 100;
    #[pyattr]
    const ALERT_DESCRIPTION_MISSING_EXTENSION: i32 = 109;
    #[pyattr]
    const ALERT_DESCRIPTION_UNSUPPORTED_EXTENSION: i32 = 110;
    #[pyattr]
    const ALERT_DESCRIPTION_CERTIFICATE_UNOBTAINABLE: i32 = 111;
    #[pyattr]
    const ALERT_DESCRIPTION_UNRECOGNIZED_NAME: i32 = 112;
    #[pyattr]
    const ALERT_DESCRIPTION_BAD_CERTIFICATE_STATUS_RESPONSE: i32 = 113;
    #[pyattr]
    const ALERT_DESCRIPTION_BAD_CERTIFICATE_HASH_VALUE: i32 = 114;
    #[pyattr]
    const ALERT_DESCRIPTION_UNKNOWN_PSK_IDENTITY: i32 = 115;
    #[pyattr]
    const ALERT_DESCRIPTION_CERTIFICATE_REQUIRED: i32 = 116;
    #[pyattr]
    const ALERT_DESCRIPTION_NO_APPLICATION_PROTOCOL: i32 = 120;

    #[pyattr]
    const OPENSSL_VERSION_NUMBER: i32 = 0x30300000;
    #[pyattr]
    const OPENSSL_VERSION: &str = "OpenSSL 3.3.0-compatible (rustls-free wasm)";
    #[pyattr]
    const OPENSSL_VERSION_INFO: (i32, i32, i32, i32, i32) = (3, 3, 0, 0, 15);
    #[pyattr]
    const _OPENSSL_API_VERSION: (i32, i32, i32, i32, i32) = (3, 3, 0, 0, 15);

    #[pyattr(once)]
    fn _DEFAULT_CIPHERS(_vm: &VirtualMachine) -> String {
        "DEFAULT".to_owned()
    }

    #[pyattr]
    const HAS_SNI: bool = true;
    #[pyattr]
    const HAS_TLS_UNIQUE: bool = false;
    #[pyattr]
    const HAS_ECDH: bool = false;
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
    const HAS_TLSv1_2: bool = false;
    #[pyattr]
    const HAS_TLSv1_3: bool = false;
    #[pyattr]
    const HAS_PHA: bool = false;

    #[pyattr]
    const ENCODING_PEM: i32 = 1;
    #[pyattr]
    const ENCODING_DER: i32 = 2;
    #[pyattr]
    const ENCODING_PEM_AUX: i32 = 0x101;

    fn ssl_error<T>(vm: &VirtualMachine, message: impl Into<String>) -> PyResult<T> {
        Err(vm
            .new_os_subtype_error(
                PySSLError::class(&vm.ctx).to_owned(),
                Some(SSL_ERROR_SSL),
                message.into(),
            )
            .upcast())
    }

    fn validate_hostname(hostname: &str, vm: &VirtualMachine) -> PyResult<()> {
        use rustpython_host_env::ssl::HostnameError;
        match rustpython_host_env::ssl::validate_hostname(hostname) {
            Ok(()) => Ok(()),
            Err(HostnameError::EmbeddedNul) => Err(exceptions::nul_char_type_error(vm)),
            Err(error) => Err(vm.new_value_error(error.message().to_owned())),
        }
    }

    fn parse_length_prefixed_alpn(bytes: &[u8], vm: &VirtualMachine) -> PyResult<Vec<Vec<u8>>> {
        rustpython_host_env::ssl::parse_length_prefixed_alpn(bytes)
            .map_err(|error| vm.new_value_error(error.0))
    }

    #[pyattr]
    #[pyclass(name = "_SSLContext", module = "ssl", traverse)]
    #[derive(Debug, PyPayload)]
    struct PySSLContext {
        #[pytraverse(skip)]
        protocol: i32,
        #[pytraverse(skip)]
        check_hostname: PyRwLock<bool>,
        #[pytraverse(skip)]
        host_flags: PyRwLock<i32>,
        #[pytraverse(skip)]
        verify_mode: PyRwLock<i32>,
        #[pytraverse(skip)]
        verify_flags: PyRwLock<i32>,
        #[pytraverse(skip)]
        options: PyRwLock<i32>,
        #[pytraverse(skip)]
        alpn_protocols: PyRwLock<Vec<Vec<u8>>>,
        #[pytraverse(skip)]
        post_handshake_auth: PyRwLock<bool>,
        #[pytraverse(skip)]
        num_tickets: PyRwLock<i32>,
        #[pytraverse(skip)]
        minimum_version: PyRwLock<i32>,
        #[pytraverse(skip)]
        maximum_version: PyRwLock<i32>,
        sni_callback: PyRwLock<Option<PyObjectRef>>,
        msg_callback: PyRwLock<Option<PyObjectRef>>,
        keylog_filename: PyRwLock<Option<PyObjectRef>>,
    }

    #[pyclass(with(Constructor, Representable), flags(BASETYPE))]
    impl PySSLContext {
        #[pygetset]
        fn check_hostname(&self) -> bool {
            *self.check_hostname.read()
        }

        #[pygetset(setter)]
        fn set_check_hostname(&self, value: bool) {
            *self.check_hostname.write() = value;
            if value && *self.verify_mode.read() == CERT_NONE {
                *self.verify_mode.write() = CERT_REQUIRED;
            }
        }

        #[pygetset]
        fn _host_flags(&self) -> i32 {
            *self.host_flags.read()
        }

        #[pygetset(setter)]
        fn set__host_flags(&self, value: i32) {
            *self.host_flags.write() = value;
        }

        #[pygetset]
        fn verify_mode(&self) -> i32 {
            *self.verify_mode.read()
        }

        #[pygetset(setter)]
        fn set_verify_mode(&self, mode: i32, vm: &VirtualMachine) -> PyResult<()> {
            if !(CERT_NONE..=CERT_REQUIRED).contains(&mode) {
                return Err(vm.new_value_error("invalid verify mode"));
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
        fn protocol(&self) -> i32 {
            self.protocol
        }

        #[pygetset]
        fn verify_flags(&self) -> i32 {
            *self.verify_flags.read()
        }

        #[pygetset(setter)]
        fn set_verify_flags(&self, value: i32) {
            *self.verify_flags.write() = value;
        }

        #[pygetset]
        fn post_handshake_auth(&self) -> bool {
            *self.post_handshake_auth.read()
        }

        #[pygetset(setter)]
        fn set_post_handshake_auth(&self, value: bool) {
            *self.post_handshake_auth.write() = value;
        }

        #[pygetset]
        fn num_tickets(&self) -> i32 {
            *self.num_tickets.read()
        }

        #[pygetset(setter)]
        fn set_num_tickets(&self, value: i32, vm: &VirtualMachine) -> PyResult<()> {
            if value < 0 {
                return Err(vm.new_value_error("num_tickets must be a non-negative integer"));
            }
            if self.protocol != PROTOCOL_TLS_SERVER {
                return Err(
                    vm.new_value_error("num_tickets can only be set on server-side contexts")
                );
            }
            *self.num_tickets.write() = value;
            Ok(())
        }

        #[pygetset]
        fn options(&self) -> i32 {
            *self.options.read()
        }

        #[pygetset(setter)]
        fn set_options(&self, value: i32, vm: &VirtualMachine) -> PyResult<()> {
            if value < 0 {
                return Err(vm.new_value_error("options must be non-negative"));
            }
            let opt_no = OP_NO_SSLv2
                | OP_NO_SSLv3
                | OP_NO_TLSv1
                | OP_NO_TLSv1_1
                | OP_NO_TLSv1_2
                | OP_NO_TLSv1_3;
            let old_opts = *self.options.read();
            let set = !old_opts & value;
            if (set & opt_no) != 0 {
                _warnings::warn(
                    vm.ctx.exceptions.deprecation_warning,
                    "ssl.OP_NO_SSL*/ssl.OP_NO_TLS* options are deprecated".to_owned(),
                    2,
                    vm,
                )?;
            }
            *self.options.write() = value;
            Ok(())
        }

        #[pygetset]
        fn minimum_version(&self) -> i32 {
            let v = *self.minimum_version.read();
            if v == 0 { PROTO_MINIMUM_SUPPORTED } else { v }
        }

        #[pygetset(setter)]
        fn set_minimum_version(&self, value: i32, vm: &VirtualMachine) -> PyResult<()> {
            if value != 0
                && value != -2
                && value != -1
                && !(PROTO_SSLv3..=PROTO_TLSv1_3).contains(&value)
            {
                return Err(vm.new_value_error(format!("invalid protocol version: {value}")));
            }
            *self.minimum_version.write() = value;
            Ok(())
        }

        #[pygetset]
        fn maximum_version(&self) -> i32 {
            let v = *self.maximum_version.read();
            if v == 0 { PROTO_MAXIMUM_SUPPORTED } else { v }
        }

        #[pygetset(setter)]
        fn set_maximum_version(&self, value: i32, vm: &VirtualMachine) -> PyResult<()> {
            if value != 0
                && value != -2
                && value != -1
                && !(PROTO_SSLv3..=PROTO_TLSv1_3).contains(&value)
            {
                return Err(vm.new_value_error(format!("invalid protocol version: {value}")));
            }
            *self.maximum_version.write() = value;
            Ok(())
        }

        #[pygetset]
        fn sni_callback(&self) -> Option<PyObjectRef> {
            self.sni_callback.read().clone()
        }

        #[pygetset(setter)]
        fn set_sni_callback(
            &self,
            callback: Option<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            if let Some(ref cb) = callback
                && !cb.is(vm.ctx.types.none_type)
                && !cb.is_callable()
            {
                return Err(vm.new_type_error("sni_callback must be callable or None"));
            }
            *self.sni_callback.write() = callback;
            Ok(())
        }

        #[pymethod]
        fn set_servername_callback(
            &self,
            callback: Option<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            self.set_sni_callback(callback, vm)
        }

        #[pygetset]
        fn security_level(&self) -> i32 {
            2
        }

        #[pygetset]
        fn _msg_callback(&self) -> Option<PyObjectRef> {
            self.msg_callback.read().clone()
        }

        #[pygetset(setter)]
        fn set__msg_callback(
            &self,
            callback: Option<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            if let Some(ref cb) = callback
                && !cb.is(vm.ctx.types.none_type)
                && !cb.is_callable()
            {
                return Err(vm.new_type_error("msg_callback must be callable or None"));
            }
            *self.msg_callback.write() = callback;
            Ok(())
        }

        #[pygetset]
        fn keylog_filename(&self) -> Option<PyObjectRef> {
            self.keylog_filename.read().clone()
        }

        #[pygetset(setter)]
        fn set_keylog_filename(&self, value: PySetterValue, vm: &VirtualMachine) -> PyResult<()> {
            let PySetterValue::Assign(value) = value else {
                return Err(vm.new_attribute_error("attribute 'keylog_filename' cannot be deleted"));
            };
            if vm.is_none(&value) {
                *self.keylog_filename.write() = None;
                return Ok(());
            }
            *self.keylog_filename.write() = Some(value);
            Ok(())
        }

        #[pymethod]
        fn _set_alpn_protocols(&self, protos: ArgBytesLike, vm: &VirtualMachine) -> PyResult<()> {
            let bytes = protos.borrow_buf();
            let alpn_list = parse_length_prefixed_alpn(&bytes, vm)?;
            *self.alpn_protocols.write() = alpn_list;
            Ok(())
        }

        #[pymethod]
        fn set_default_verify_paths(&self, _vm: &VirtualMachine) -> PyResult<()> {
            Ok(())
        }

        #[pymethod]
        fn load_default_certs(&self, _vm: &VirtualMachine) -> PyResult<()> {
            Ok(())
        }

        #[pymethod]
        fn load_verify_locations(
            &self,
            args: LoadVerifyLocationsArgs,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let has_cafile = matches!(&args.cafile, OptionalArg::Present(Some(_)));
            let has_capath = matches!(&args.capath, OptionalArg::Present(Some(_)));
            let has_cadata = matches!(&args.cadata, OptionalArg::Present(Some(_)));
            if !has_cafile && !has_capath && !has_cadata {
                return Err(vm.new_type_error("cafile, capath and cadata cannot be all omitted"));
            }
            Ok(())
        }

        #[pymethod]
        fn load_cert_chain(&self, _args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
            ssl_error(vm, UNAVAILABLE)
        }

        #[pymethod]
        fn set_ciphers(&self, _ciphers: PyUtf8StrRef, vm: &VirtualMachine) -> PyResult<()> {
            ssl_error(vm, UNAVAILABLE)
        }

        #[pymethod]
        fn set_ecdh_curve(&self, _name: PyUtf8StrRef, vm: &VirtualMachine) -> PyResult<()> {
            ssl_error(vm, UNAVAILABLE)
        }

        #[pymethod]
        fn load_dh_params(&self, _filepath: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            ssl_error(vm, UNAVAILABLE)
        }

        #[pymethod]
        fn get_ciphers(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            Ok(vm.ctx.new_list(Vec::new()).into())
        }

        #[pymethod]
        fn get_ca_certs(&self, args: GetCertArgs, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            let _ = args.binary_form;
            Ok(vm.ctx.new_list(Vec::new()).into())
        }

        #[pymethod]
        fn cert_store_stats(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            let dict = vm.ctx.new_dict();
            dict.set_item("x509", vm.ctx.new_int(0).into(), vm)?;
            dict.set_item("crl", vm.ctx.new_int(0).into(), vm)?;
            dict.set_item("x509_ca", vm.ctx.new_int(0).into(), vm)?;
            Ok(dict.into())
        }

        #[pymethod]
        fn session_stats(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            let dict = vm.ctx.new_dict();
            dict.set_item("number", vm.ctx.new_int(0).into(), vm)?;
            dict.set_item("connect", vm.ctx.new_int(0).into(), vm)?;
            dict.set_item("connect_good", vm.ctx.new_int(0).into(), vm)?;
            dict.set_item("connect_renegotiate", vm.ctx.new_int(0).into(), vm)?;
            dict.set_item("accept", vm.ctx.new_int(0).into(), vm)?;
            dict.set_item("accept_good", vm.ctx.new_int(0).into(), vm)?;
            dict.set_item("accept_renegotiate", vm.ctx.new_int(0).into(), vm)?;
            dict.set_item("hits", vm.ctx.new_int(0).into(), vm)?;
            dict.set_item("misses", vm.ctx.new_int(0).into(), vm)?;
            dict.set_item("timeouts", vm.ctx.new_int(0).into(), vm)?;
            dict.set_item("cache_full", vm.ctx.new_int(0).into(), vm)?;
            Ok(dict.into())
        }

        #[pymethod]
        fn _wrap_socket(&self, _args: FuncArgs, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            ssl_error(vm, UNAVAILABLE)
        }

        #[pymethod]
        fn _wrap_bio(&self, args: WrapBioArgs, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            if let Some(hostname) = args.server_hostname.into_option().flatten() {
                validate_hostname(hostname.as_str(), vm)?;
            }
            if args.server_side.unwrap_or(false) && self.protocol == PROTOCOL_TLS_CLIENT {
                return ssl_error(
                    vm,
                    "Cannot create a server socket with a PROTOCOL_TLS_CLIENT context",
                );
            }
            if !args.server_side.unwrap_or(false) && self.protocol == PROTOCOL_TLS_SERVER {
                return ssl_error(
                    vm,
                    "Cannot create a client socket with a PROTOCOL_TLS_SERVER context",
                );
            }
            ssl_error(vm, UNAVAILABLE)
        }
    }

    impl Representable for PySSLContext {
        #[inline]
        fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            Ok(format!("<SSLContext(protocol={})>", zelf.protocol))
        }
    }

    impl Constructor for PySSLContext {
        type Args = (i32,);

        fn py_new(
            _cls: &Py<PyType>,
            (protocol,): Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult<Self> {
            let deprecated_protocol = match protocol {
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
            if let Some(protocol_name) = deprecated_protocol {
                _warnings::warn(
                    vm.ctx.exceptions.deprecation_warning,
                    format!("ssl.{protocol_name} is deprecated"),
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
            let default_verify_mode = if protocol == PROTOCOL_TLS_CLIENT {
                CERT_REQUIRED
            } else {
                CERT_NONE
            };
            let (min_version, max_version) = match protocol {
                PROTOCOL_TLSv1_2 => (PROTO_TLSv1_2, PROTO_TLSv1_2),
                PROTOCOL_TLSv1_3 => (PROTO_TLSv1_3, PROTO_TLSv1_3),
                _ => (PROTO_MINIMUM_SUPPORTED, PROTO_MAXIMUM_SUPPORTED),
            };

            Ok(Self {
                protocol,
                check_hostname: PyRwLock::new(protocol == PROTOCOL_TLS_CLIENT),
                host_flags: PyRwLock::new(0),
                verify_mode: PyRwLock::new(default_verify_mode),
                verify_flags: PyRwLock::new(VERIFY_DEFAULT | VERIFY_X509_TRUSTED_FIRST),
                options: PyRwLock::new(default_options),
                alpn_protocols: PyRwLock::new(Vec::new()),
                post_handshake_auth: PyRwLock::new(false),
                num_tickets: PyRwLock::new(2),
                minimum_version: PyRwLock::new(min_version),
                maximum_version: PyRwLock::new(max_version),
                sni_callback: PyRwLock::new(None),
                msg_callback: PyRwLock::new(None),
                keylog_filename: PyRwLock::new(None),
            })
        }
    }

    #[derive(FromArgs)]
    struct GetCertArgs {
        #[pyarg(any, optional)]
        binary_form: OptionalArg<bool>,
    }

    #[derive(FromArgs)]
    struct LoadVerifyLocationsArgs {
        #[pyarg(any, optional)]
        cafile: OptionalArg<Option<PyObjectRef>>,
        #[pyarg(any, optional)]
        capath: OptionalArg<Option<PyObjectRef>>,
        #[pyarg(any, optional)]
        cadata: OptionalArg<Option<PyObjectRef>>,
    }

    #[derive(FromArgs)]
    struct WrapBioArgs {
        #[allow(dead_code)]
        incoming: PyRef<PyMemoryBIO>,
        #[allow(dead_code)]
        outgoing: PyRef<PyMemoryBIO>,
        #[pyarg(named, optional)]
        server_side: OptionalArg<bool>,
        #[pyarg(named, optional)]
        server_hostname: OptionalArg<Option<PyUtf8StrRef>>,
        #[pyarg(named, optional)]
        #[allow(dead_code)]
        owner: OptionalArg<PyObjectRef>,
        #[pyarg(named, optional)]
        #[allow(dead_code)]
        session: OptionalArg<PyObjectRef>,
    }

    #[pyattr]
    #[pyclass(name = "MemoryBIO", module = "ssl")]
    #[derive(Debug, PyPayload)]
    struct PyMemoryBIO {
        inner: PyMutex<rustpython_host_env::ssl::MemoryBio>,
    }

    #[pyclass(with(Constructor, Representable), flags(BASETYPE))]
    impl PyMemoryBIO {
        #[pymethod]
        fn read(&self, len: OptionalArg<i32>, vm: &VirtualMachine) -> PyResult<PyBytesRef> {
            let mut bio = self.inner.lock();
            let read_len = match len {
                OptionalArg::Present(n) if n >= 0 => n as usize,
                OptionalArg::Present(_) | OptionalArg::Missing => bio.pending(),
            };
            Ok(vm.ctx.new_bytes(bio.read(read_len)))
        }

        #[pymethod]
        fn write(&self, buf: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
            if let Ok(mem_view) = buf.get_attr("c_contiguous", vm) {
                let is_contiguous: bool = mem_view.try_to_bool(vm)?;
                if !is_contiguous {
                    return Err(vm.new_buffer_error("non-contiguous buffer is not supported"));
                }
            }
            let bytes_like = ArgBytesLike::try_from_object(vm, buf)?;
            let data = bytes_like.borrow_buf();
            self.inner.lock().write(&data).map_err(|err| {
                vm.new_os_subtype_error(
                    PySSLError::class(&vm.ctx).to_owned(),
                    None,
                    err.to_string(),
                )
                .upcast()
            })
        }

        #[pymethod]
        fn write_eof(&self, _vm: &VirtualMachine) {
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

    impl Representable for PyMemoryBIO {
        #[inline]
        fn repr_str(_zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            Ok("<MemoryBIO>".to_owned())
        }
    }

    impl Constructor for PyMemoryBIO {
        type Args = ();

        fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
            Ok(Self {
                inner: PyMutex::new(rustpython_host_env::ssl::MemoryBio::new()),
            })
        }
    }

    #[pyattr]
    #[pyclass(name = "SSLSession", module = "ssl")]
    #[derive(Debug, PyPayload)]
    struct PySSLSession {
        session_id: Vec<u8>,
        lifetime: u64,
        has_ticket: bool,
    }

    #[pyclass(flags(BASETYPE), with(Comparable, Representable))]
    impl PySSLSession {
        #[pygetset]
        fn time(&self) -> i64 {
            0
        }

        #[pygetset]
        fn timeout(&self) -> i64 {
            self.lifetime as i64
        }

        #[pygetset]
        fn ticket_lifetime_hint(&self) -> i64 {
            self.lifetime as i64
        }

        #[pygetset]
        fn id(&self, vm: &VirtualMachine) -> PyBytesRef {
            vm.ctx.new_bytes(self.session_id.clone())
        }

        #[pygetset]
        fn has_ticket(&self) -> bool {
            self.has_ticket
        }
    }

    impl Comparable for PySSLSession {
        fn cmp(
            zelf: &Py<Self>,
            other: &PyObject,
            op: PyComparisonOp,
            _vm: &VirtualMachine,
        ) -> PyResult<PyComparisonValue> {
            op.eq_only(|| {
                if let Some(other_session) = other.downcast_ref::<Self>() {
                    Ok((zelf.session_id == other_session.session_id).into())
                } else {
                    Ok(PyComparisonValue::NotImplemented)
                }
            })
        }
    }

    impl Representable for PySSLSession {
        #[inline]
        fn repr_str(_zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            Ok("<SSLSession>".to_owned())
        }
    }

    #[derive(FromArgs)]
    struct Txt2ObjArgs {
        txt: PyUtf8StrRef,
        #[pyarg(named, optional)]
        name: OptionalArg<bool>,
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
        let name = args.name.unwrap_or(false);
        let entry = if txt.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            oid::find_by_oid_string(txt)
        } else if name {
            oid::find_by_name(txt)
        } else {
            None
        };
        let entry = entry.ok_or_else(|| vm.new_value_error(format!("unknown object '{txt}'")))?;
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
                vm.ctx.new_str("").into(),
                vm.ctx.new_str("SSL_CERT_DIR").into(),
                vm.ctx.new_str("").into(),
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
        getrandom::fill(&mut buf)
            .map_err(|_| vm.new_os_error("Failed to generate random bytes"))?;
        Ok(PyBytesRef::from(vm.ctx.new_bytes(buf)))
    }

    #[pyfunction]
    fn RAND_pseudo_bytes(n: i32, vm: &VirtualMachine) -> PyResult<(PyBytesRef, bool)> {
        let bytes = RAND_bytes(n, vm)?;
        Ok((bytes, true))
    }
}
