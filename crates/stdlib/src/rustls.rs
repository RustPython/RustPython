// spell-checker: ignore ders SUITEB COMPLEMENTOFDEFAULT COMPLEMENTOFALL AESGCM MLKEM nids

//! SSL/TLS implementation using rustls
//!
//! Warning: This module implements security primitives and it was not audited properly.
//!
//! Warning: This module still contains LLM-generated code.
//!
//! cpython's original ssl module was designed around OpenSSL and thus tightly coupled to
//! OpenSSL API and internals. 100% compatible re-implementation using any other SSL/TLS library
//! is near to impossible.
//!
//! This module uses `rustls` to provide a "best effort" compatibility with original `ssl`
//! implementation. In particular:
//!   * Security-related functionality that is not supported by `rustls` is not implemented
//!     and raises errors.
//!   * Most of the SSLContext.options are not supported, set to zero and thus ignored.
//!     All unsupported options are either irrelevant to security or meant to lower it.
//!   * `rustls` is designed to be safe to use by default. However, it does not perform
//!     all the certificate checks that OpenSSL does when VERIFY_X509_STRICT is enabled.
//!     Unfortunately, a some client code may set VERIFY_X509_STRICT by default so we have to silently
//!     ignore it.
//!   * To support verifying certificates with both "default" certificate stores
//!     (`SSLContext.load_default_certs()`) and provided root certificates
//!     (`SSLContext.load_verify_locations()`) this implementation uses combined certificate
//!     verifier consisting of `rustls_platform_verifier::Verifier` and `WebPkiServerVerifier`.
//!     Combined certificate verifier reports certificates as valid when at least one of the underlying
//!     verifiers reports it as valid and all others report "unknown issuer".
//!     CRL verification control is unreliable with `SSLContext.load_default_certs()` because
//!     `rustls_platform_verifier::Verifier` does not have settings for this and CRL support
//!     varies by platform.
//!   * Exposing TLS sessions to client code is not supported, dummy value returned. See comments inside
//!     `PySSLSocket::set_session()`. Session resumption works out of the box.
//!   * Channel binding are not supported and raises error. See comments inside `PySSLSocket::get_channel_binding()`.
//!   * Post-handshake authentication is not supported, `SSLSocket.verify_client_post_handshake()` raises an error.
//!   * SSLContext.hostname_checks_common_name is ignored because `rustls` always uses alt names to check server name.

use alloc::{rc::Rc, sync::Arc};
use core::{
    net::Ipv6Addr,
    str::FromStr,
    sync::atomic::{AtomicUsize, Ordering},
};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use base64::{Engine, prelude::BASE64_STANDARD};
use chrono::{DateTime, Utc};
use pkcs8::{EncryptedPrivateKeyInfoRef, PrivateKeyInfoRef, der::Decode};
use rustls::{
    CipherSuite, Connection, DigitallySignedStruct, DistinguishedName, ProtocolVersion,
    RootCertStore, SignatureScheme, SupportedCipherSuite,
    client::WebPkiServerVerifier,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::{CryptoProvider, SupportedKxGroup},
    server::{AcceptedAlert, Acceptor},
};
use rustls_pki_types::{
    CertificateDer, CertificateRevocationListDer, DnsName, IpAddr, Ipv4Addr, ServerName, UnixTime,
};
use serde::{Serialize, Serializer};
use sha2::{Digest, Sha256};
use x509_parser::{
    extensions::{DistributionPointName, GeneralName, ParsedExtension},
    oid_registry::{
        OID_PKIX_ACCESS_DESCRIPTOR_CA_ISSUERS, OID_PKIX_ACCESS_DESCRIPTOR_OCSP,
        OID_PKIX_AUTHORITY_INFO_ACCESS, OID_X509_EXT_CRL_DISTRIBUTION_POINTS, Oid, OidEntry,
        OidRegistry,
    },
    parse_x509_certificate, parse_x509_crl,
    pem::Pem,
    time::ASN1Time,
    x509::X509Name,
};

use crate::{
    common::lock::LazyLock,
    vm::{
        AsObject as _, PyObjectRef, PyResult, TryFromObject, VirtualMachine,
        builtins::{PyBaseExceptionRef, PyTupleRef, PyUtf8StrRef},
        convert::{IntoObject, RustPySerDeConf},
        function::{ArgBytesLike, OptionalArg},
    },
};

#[path = "ssl/compat.rs"]
mod compat;
// SSL exception types (shared with openssl backend)
#[path = "ssl/error.rs"]
mod error;
#[path = "ssl/providers.rs"]
pub mod providers;

// TODO: SslError should not convert errors to strings to check the type.
use compat::{SslError, SslResult};
use providers::CryptoExt;

pub(crate) use _ssl::module_def;

#[allow(non_snake_case)]
#[allow(non_upper_case_globals)]
#[pymodule(with(error::ssl_error))]
mod _ssl {
    use alloc::sync::Arc;
    use core::{
        hash::{Hash as _, Hasher as _},
        slice,
        sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering},
    };
    use std::{
        hash::DefaultHasher,
        io::{BufRead, Read, Write},
        time::{SystemTime, UNIX_EPOCH},
    };

    use itertools::Itertools as _;
    use rustls::{
        ALL_VERSIONS, ClientConfig, ClientConnection, Connection, HandshakeKind, ProtocolVersion,
        ServerConfig, SupportedCipherSuite, SupportedProtocolVersion,
        client::Resumption,
        crypto::{CryptoProvider, SupportedKxGroup},
        server::{
            Accepted, AcceptedAlert, NoClientAuth, WebPkiClientVerifier, danger::ClientCertVerifier,
        },
        sign::CertifiedKey,
    };
    use rustls_pki_types::{CertificateDer, IpAddr, Ipv4Addr, PrivateKeyDer, ServerName};
    use serde::Serialize as _;
    use x509_parser::{oid_registry::Oid, parse_x509_certificate};

    use crate::{
        common::{
            hash::PyHash,
            lock::{PyMutex, PyRwLock},
        },
        vm::{
            AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject,
            VirtualMachine,
            builtins::{
                PyBytesRef, PyListRef, PyModule, PyStrRef, PyTupleRef, PyType, PyTypeRef,
                PyUtf8StrRef,
            },
            convert::{IntoObject, IntoPyException},
            function::{
                ArgBytesLike, ArgMemoryBuffer, Either, FsPath, FuncArgs, OptionalArg,
                PyComparisonValue,
            },
            object::PyWeak,
            stdlib::_warnings,
            types::{Comparable, Constructor, Hashable, PyComparisonOp, Representable},
        },
    };

    use super::{
        CIPHER_MAPPINGS, CertInfo, CertStore, CipherDescriptionDict, CipherList, CloseNotifyState,
        ConnectionState, CrlCheck, CustomServerCertVerifier, DerKind, Io, OID_MAPPINGS, Password,
        SECURITY_LEVEL_TO_MIN_BITS, State, Stats, WithOptionSuiteB, cipher_to_tuple,
        cipher_to_version, compat::SslError, der_to_pem_cert, ensure_single_der_bytes,
        load_der_bytes_from_der, load_der_bytes_from_pem, load_der_bytes_from_pem_or_der_bytes,
        load_der_bytes_from_pem_or_der_file, providers::CryptoExt,
    };

    #[expect(clippy::unnecessary_wraps, reason = "pymodule hook expects PyResult")]
    pub(crate) fn module_exec(vm: &VirtualMachine, module: &Py<PyModule>) -> PyResult<()> {
        __module_exec(vm, module);
        vm.register_module_loaded_hook("ssl", patch_ssl_module);
        Ok(())
    }

    fn patch_ssl_module(vm: &VirtualMachine, module: PyObjectRef) -> PyResult<()> {
        const CHANNEL_BINDING_TYPES: &str = "CHANNEL_BINDING_TYPES";
        // Check that it already exists and is a list.
        let _ = PyListRef::try_from_object(vm, module.get_attr(CHANNEL_BINDING_TYPES, vm)?)?;
        // rustls does not support OpenSSL's channel bindings.
        // See SSLSocket::get_channel_binding() for details.
        module.set_attr(CHANNEL_BINDING_TYPES, vm.ctx.new_list(vec![]), vm)
    }

    // Constants matching Python ssl module

    // SSL/TLS Protocol versions
    #[pyattr]
    const PROTOCOL_TLS: i32 = 2; // Auto-negotiate best version
    #[pyattr]
    const PROTOCOL_SSLv23: i32 = PROTOCOL_TLS;
    #[pyattr]
    const PROTOCOL_TLS_CLIENT: i32 = 16;
    #[pyattr]
    const PROTOCOL_TLS_SERVER: i32 = 17;

    // Note: rustls doesn't support TLS 1.0/1.1 for security reasons
    // These are defined for API compatibility but will raise errors if used
    #[pyattr]
    const PROTOCOL_TLSv1: i32 = 3;
    #[pyattr]
    const PROTOCOL_TLSv1_1: i32 = 4;
    #[pyattr]
    const PROTOCOL_TLSv1_2: i32 = 5;
    #[pyattr]
    const PROTOCOL_TLSv1_3: i32 = 6;

    // Protocol version constants for TLSVersion enum
    #[pyattr]
    const PROTO_SSLv3: i32 = 0x0300;
    #[pyattr]
    const PROTO_TLSv1: i32 = 0x0301;
    #[pyattr]
    const PROTO_TLSv1_1: i32 = 0x0302;
    #[pyattr]
    const PROTO_TLSv1_2: i32 = 0x0303;
    #[pyattr]
    const PROTO_TLSv1_3: i32 = 0x0304;

    // Minimum and maximum supported protocol versions for rustls
    #[pyattr]
    const PROTO_MINIMUM_SUPPORTED: i32 = -2; // special value
    #[pyattr]
    const PROTO_MAXIMUM_SUPPORTED: i32 = -1; // special value

    // Certificate verification modes
    #[pyattr]
    const CERT_NONE: i32 = 0;
    #[pyattr]
    const CERT_OPTIONAL: i32 = 1;
    #[pyattr]
    const CERT_REQUIRED: i32 = 2;

    // SSL Verification Flags / Certificate requirements
    #[pyattr]
    const VERIFY_DEFAULT: i32 = 0x00000000;
    #[pyattr]
    const VERIFY_CRL_CHECK_LEAF: i32 = super::VERIFY_CRL_CHECK_LEAF;
    #[pyattr]
    const VERIFY_CRL_CHECK_CHAIN: i32 = super::VERIFY_CRL_CHECK_CHAIN;
    // rustls strictly verifies certificates by default but does not do some checks that
    // OpenSSL does in this mode (Authority Key Identifier verification, for example).
    // We have to ignore this because a lot of clients set this by default.
    #[pyattr]
    const VERIFY_X509_STRICT: i32 = 0x00000000;
    #[pyattr]
    const VERIFY_ALLOW_PROXY_CERTS: i32 = 0x00000000; // not supported by rustls
    #[pyattr]
    const VERIFY_X509_TRUSTED_FIRST: i32 = 0x00000000; // this is the default behaviour and is not configurable in rustls
    #[pyattr]
    const VERIFY_X509_PARTIAL_CHAIN: i32 = 0x00000000; // not supported by rustls

    // Options (OpenSSL-compatible flags, mostly no-op in rustls)
    #[pyattr]
    const OP_NO_SSLv2: i32 = 0x00000000; // rustls does not support SSLv2.0
    #[pyattr]
    const OP_NO_SSLv3: i32 = 0x00000000; // rustls does not support SSLv3.0
    #[pyattr]
    const OP_NO_TLSv1: i32 = 0x00000000; // rustls does not support TLSv1.0
    #[pyattr]
    const OP_NO_TLSv1_1: i32 = 0x00000000; // rustls does not support TLSv1.1
    #[pyattr]
    const OP_NO_TLSv1_2: i32 = 0x08000000;
    #[pyattr]
    const OP_NO_TLSv1_3: i32 = 0x20000000;
    #[pyattr]
    const OP_NO_COMPRESSION: i32 = 0x00000000; // rustls does not support compression
    #[pyattr]
    const OP_CIPHER_SERVER_PREFERENCE: i32 = 0x00400000;
    #[pyattr]
    const OP_SINGLE_DH_USE: i32 = 0x00000000; // rustls does not support Diffie-Hellman key exchange
    #[pyattr]
    const OP_SINGLE_ECDH_USE: i32 = 0x00000000; // rustls does not reuse ECDHE keys by default
    #[pyattr]
    const OP_NO_TICKET: i32 = 0x00004000;
    #[pyattr]
    const OP_LEGACY_SERVER_CONNECT: i32 = 0x00000000; // rustls does not support this
    #[pyattr]
    const OP_NO_RENEGOTIATION: i32 = 0x00000000; // rustls does not support renegotiation
    // TODO: Should be easy to support. But it lowers security and we might just ignore it.
    #[pyattr]
    const OP_IGNORE_UNEXPECTED_EOF: i32 = 0x00000000;
    #[pyattr]
    const OP_ENABLE_MIDDLEBOX_COMPAT: i32 = 0x00000000; // rustls does not support this
    // Reflect what rustls supports
    #[pyattr]
    // | OP_NO_SSLv3 | OP_ENABLE_MIDDLEBOX_COMPAT
    const OP_ALL: i32 = OP_CIPHER_SERVER_PREFERENCE;

    // Alert types (matching _TLSAlertType enum)
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

    // Version info - reporting as OpenSSL 3.3.0 for compatibility
    #[pyattr]
    const OPENSSL_VERSION_NUMBER: i32 = 0x30300000; // OpenSSL 3.3.0
    // TODO: Add version of rustls, used cryptography provider and enabled features here.
    #[pyattr]
    const OPENSSL_VERSION: &str = "OpenSSL 3.3.0 (rustls)";
    #[pyattr]
    const OPENSSL_VERSION_INFO: (i32, i32, i32, i32, i32) = (3, 3, 0, 0, 15); // 3.3.0 release
    #[pyattr]
    const _OPENSSL_API_VERSION: (i32, i32, i32, i32, i32) = (3, 3, 0, 0, 15); // 3.3.0 release

    #[pyattr(once)]
    fn _DEFAULT_CIPHERS(_vm: &VirtualMachine) -> String {
        CIPHER_MAPPINGS
            .default
            .iter()
            .map(|id| CIPHER_MAPPINGS.id_to_openssl[id])
            .join(":")
    }

    // Has features
    #[pyattr]
    const HAS_SNI: bool = true;
    #[pyattr]
    const HAS_TLS_UNIQUE: bool = false; // Not supported in rustls
    #[pyattr]
    const HAS_ECDH: bool = true;
    #[pyattr]
    const HAS_NPN: bool = false; // Deprecated, not supported in rustls use ALPN
    #[pyattr]
    const HAS_ALPN: bool = true;
    #[pyattr]
    const HAS_PSK: bool = false; // PSK not supported in rustls
    #[pyattr]
    const HAS_SSLv2: bool = false; // Not supported in rustls for security
    #[pyattr]
    const HAS_SSLv3: bool = false; // Not supported in rustls for security
    #[pyattr]
    const HAS_TLSv1: bool = false; // Not supported in rustls for security
    #[pyattr]
    const HAS_TLSv1_1: bool = false; // Not supported in rustls for security
    #[pyattr]
    const HAS_TLSv1_2: bool = true;
    #[pyattr]
    const HAS_TLSv1_3: bool = true;
    #[pyattr]
    const HAS_PHA: bool = false; // Post-Handshake Auth not supported in rustls

    // Encoding constants (matching OpenSSL)
    #[pyattr]
    const ENCODING_PEM: i32 = 1;
    #[pyattr]
    const ENCODING_DER: i32 = 2;

    #[pyattr]
    const HOSTFLAG_NEVER_CHECK_SUBJECT: i32 = 0x00000001; // rustls always uses alt names to check server name

    // Matches recent versions of OpenSSL;
    const SSL_SESSION_CACHE_MAX_SIZE_DEFAULT: usize = 1024 * 10;

    // _SSLContext - manages TLS configuration
    #[pyattr]
    #[pyclass(module = "_ssl", name = "_SSLContext")]
    #[derive(Debug, PyPayload)]
    struct PySSLContext {
        protocol: i32,
        ciphers: PyRwLock<WithOptionSuiteB<Vec<SupportedCipherSuite>>>,
        options: AtomicI32,
        ecdh_curve: PyRwLock<Option<Vec<&'static dyn SupportedKxGroup>>>,
        verify_mode: AtomicI32,
        check_hostname: AtomicBool,
        verify_flags: AtomicI32,
        num_tickets: AtomicUsize,
        minimum_version: AtomicI32,
        maximum_version: AtomicI32,
        use_system_certificates: AtomicBool,
        alpn_protocols: PyRwLock<Vec<Vec<u8>>>,
        sni_callback: PyRwLock<PyObjectRef>,
        msg_callback: PyRwLock<PyObjectRef>,
        cert_chain: PyRwLock<Vec<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)>>,
        stats: Arc<Stats>,
        cert_store: PyRwLock<CertStore>,
        post_handshake_auth: AtomicBool,
        session_cache: Resumption,
        host_flags: AtomicI32,
    }

    impl Representable for PySSLContext {
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
            // Validate protocol
            if !matches!(
                protocol,
                PROTOCOL_TLS_CLIENT
                    | PROTOCOL_TLS_SERVER
                    | PROTOCOL_TLS
                    | PROTOCOL_TLSv1_2
                    | PROTOCOL_TLSv1_3
            ) {
                return Err(
                    vm.new_value_error(format!("protocol {protocol} is not supported by rustls"))
                );
            }

            let client_protocol = protocol == PROTOCOL_TLS_CLIENT;
            let (minimum_version, maximum_version) = match protocol {
                PROTOCOL_TLSv1_2 => (PROTO_TLSv1_2, PROTO_TLSv1_2),
                PROTOCOL_TLSv1_3 => (PROTO_TLSv1_3, PROTO_TLSv1_3),
                _ => (PROTO_TLSv1_2, PROTO_TLSv1_3),
            };
            let stats = Arc::new(Stats::default());

            Ok(Self {
                protocol,
                ciphers: PyRwLock::new((
                    CryptoExt::get_ext().default_ciphers_or_provider().to_vec(),
                    None,
                )),
                options: AtomicI32::new(OP_ALL),
                ecdh_curve: PyRwLock::new(None),

                verify_mode: AtomicI32::new(if client_protocol {
                    CERT_REQUIRED
                } else {
                    CERT_NONE
                }),

                check_hostname: AtomicBool::new(client_protocol),
                verify_flags: AtomicI32::new(VERIFY_DEFAULT),
                num_tickets: AtomicUsize::new(2),
                minimum_version: AtomicI32::new(minimum_version),
                maximum_version: AtomicI32::new(maximum_version),
                use_system_certificates: AtomicBool::new(false),
                alpn_protocols: PyRwLock::new(vec![]),
                sni_callback: PyRwLock::new(vm.ctx.none()),
                msg_callback: PyRwLock::new(vm.ctx.none()),
                cert_chain: PyRwLock::new(Vec::new()),
                stats: stats.clone(),
                cert_store: PyRwLock::new(CertStore::empty(stats)),
                post_handshake_auth: AtomicBool::new(false),
                session_cache: Resumption::in_memory_sessions(SSL_SESSION_CACHE_MAX_SIZE_DEFAULT),
                host_flags: AtomicI32::new(0),
            })
        }
    }

    #[pyclass(with(Constructor, Representable), flags(BASETYPE))]
    impl PySSLContext {
        #[pygetset]
        fn protocol(&self) -> i32 {
            self.protocol
        }

        #[pymethod]
        fn set_ciphers(&self, ciphers: PyUtf8StrRef, vm: &VirtualMachine) -> PyResult<()> {
            let mut ciphers = CipherList::parse_to_rustls(ciphers.as_str()).map_err(|_| {
                SslError::Ssl("No cipher can be selected".to_string()).into_py_err(vm)
            })?;

            // TLS 1.3 cipher suites cannot be disabled with set_ciphers().
            for cipher in CryptoExt::get_ext()
                .default_ciphers_or_provider()
                .iter()
                .rev()
            {
                if cipher.tls13().is_some() && !ciphers.0.contains(cipher) {
                    // We assume that TLS 1.3 is the most secure thing possible so it should be preferred.
                    ciphers.0.insert(0, *cipher);
                }
            }

            *self.ciphers.write() = ciphers;
            Ok(())
        }

        #[pymethod]
        fn get_ciphers(&self, vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
            self.ciphers
                .read()
                .0
                .iter()
                .map(CipherDescriptionDict::new)
                .map(|c| vm.with_serde(|s| c.serialize(s)))
                .collect::<PyResult<_>>()
        }

        #[pygetset(setter)]
        fn set_options(&self, value: i32, vm: &VirtualMachine) -> PyResult<()> {
            if value < 0 {
                return Err(vm.new_value_error("options must be non-negative"));
            }

            const DEPRECATED_OPS: i32 = OP_NO_SSLv2
                | OP_NO_SSLv3
                | OP_NO_TLSv1
                | OP_NO_TLSv1_1
                | OP_NO_TLSv1_2
                | OP_NO_TLSv1_3;
            if (value & DEPRECATED_OPS) != 0 {
                _warnings::warn(
                    vm.ctx.exceptions.deprecation_warning,
                    "ssl.OP_NO_* options are deprecated".to_string(),
                    2,
                    vm,
                )?;
            }

            self.options.store(value, Ordering::Relaxed);
            Ok(())
        }

        #[pygetset]
        fn options(&self) -> i32 {
            self.options.load(Ordering::Relaxed)
        }

        #[pymethod]
        fn set_ecdh_curve(&self, name: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            let curve_name = if let Ok(s) = PyUtf8StrRef::try_from_object(vm, name.clone()) {
                s.as_str().to_owned()
            } else if let Ok(b) = ArgBytesLike::try_from_object(vm, name) {
                String::from_utf8(b.borrow_buf().to_vec())
                    .map_err(|_| vm.new_value_error("Invalid curve name encoding"))?
            } else {
                return Err(vm.new_type_error("ECDH curve name must be str or bytes"));
            };

            if let Some(ecdh_curve) = CIPHER_MAPPINGS.name_to_kx_group.get(&curve_name) {
                *self.ecdh_curve.write() = Some(vec![*ecdh_curve]);
                Ok(())
            } else {
                Err(vm.new_value_error(format!("unknown curve name '{curve_name}'")))
            }
        }

        #[pygetset(setter)]
        fn set_verify_mode(&self, mode: i32, vm: &VirtualMachine) -> PyResult<()> {
            if ![CERT_NONE, CERT_OPTIONAL, CERT_REQUIRED].contains(&mode) {
                return Err(vm.new_value_error("invalid verify mode"));
            }
            // Cannot set CERT_NONE when check_hostname is enabled
            if mode == CERT_NONE && self.check_hostname.load(Ordering::Relaxed) {
                return Err(vm.new_value_error(
                    "Cannot set verify_mode to CERT_NONE when check_hostname is enabled",
                ));
            }
            self.verify_mode.store(mode, Ordering::Relaxed);
            Ok(())
        }

        #[pygetset]
        fn verify_mode(&self) -> i32 {
            self.verify_mode.load(Ordering::Relaxed)
        }

        #[pygetset(setter)]
        fn set_check_hostname(&self, value: bool) {
            // When check_hostname is enabled, ensure verify_mode is at least CERT_REQUIRED
            if value {
                let _ = self.verify_mode.compare_exchange(
                    CERT_NONE,
                    CERT_REQUIRED,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                );
            }
            self.check_hostname.store(value, Ordering::Relaxed);
        }

        #[pygetset]
        fn check_hostname(&self) -> bool {
            self.check_hostname.load(Ordering::Relaxed)
        }

        #[pygetset(setter)]
        fn set_num_tickets(&self, value: isize, vm: &VirtualMachine) -> PyResult<()> {
            let value = value
                .try_into()
                .map_err(|_| vm.new_value_error(format!("num_tickets is out of range: {value}")))?;

            if self.protocol != PROTOCOL_TLS_SERVER {
                return Err(
                    vm.new_value_error("num_tickets can only be set on server-side contexts")
                );
            }
            self.num_tickets.store(value, Ordering::Relaxed);
            Ok(())
        }

        #[pygetset]
        fn num_tickets(&self) -> usize {
            self.num_tickets.load(Ordering::Relaxed)
        }

        #[pygetset]
        fn minimum_version(&self) -> i32 {
            self.minimum_version.load(Ordering::Relaxed)
        }

        #[pygetset(setter)]
        fn set_minimum_version(&self, mut value: i32, vm: &VirtualMachine) -> PyResult<()> {
            value = Self::sanitize_version(value, vm)?;
            if value > self.maximum_version.load(Ordering::Relaxed) {
                Err(vm.new_value_error(
                    "new SSLContext.minimum_version is greater than SSLContext.maximum_version",
                ))
            } else {
                self.minimum_version.store(value, Ordering::Relaxed);
                Ok(())
            }
        }

        #[pygetset]
        fn maximum_version(&self) -> i32 {
            self.maximum_version.load(Ordering::Relaxed)
        }

        #[pygetset(setter)]
        fn set_maximum_version(&self, mut value: i32, vm: &VirtualMachine) -> PyResult<()> {
            value = Self::sanitize_version(value, vm)?;
            if value < self.minimum_version.load(Ordering::Relaxed) {
                Err(vm.new_value_error(
                    "new SSLContext.maximum_version is less than SSLContext.minimum_version",
                ))
            } else {
                self.maximum_version.store(value, Ordering::Relaxed);
                Ok(())
            }
        }

        fn sanitize_version(mut value: i32, vm: &VirtualMachine) -> PyResult<i32> {
            if ![
                PROTO_MINIMUM_SUPPORTED,
                PROTO_MAXIMUM_SUPPORTED,
                PROTO_SSLv3,
                PROTO_TLSv1,
                PROTO_TLSv1_1,
                PROTO_TLSv1_2,
                PROTO_TLSv1_3,
            ]
            .contains(&value)
            {
                return Err(vm.new_value_error(format!("invalid protocol version: {value}")));
            }

            if value == PROTO_MINIMUM_SUPPORTED {
                value = PROTO_TLSv1_2;
            } else if value == PROTO_MAXIMUM_SUPPORTED {
                value = PROTO_TLSv1_3;
            }

            if ![PROTO_TLSv1_2, PROTO_TLSv1_3].contains(&value) {
                return Err(vm.new_value_error(
                    "rustls only supports ssl.TLSVersion.TLSv1_2 and ssl.TLSVersion.TLSv1_3",
                ));
            }

            Ok(value)
        }

        #[pymethod]
        fn set_default_verify_paths(&self, vm: &VirtualMachine) -> PyResult<()> {
            // Check for environment variable overrides.
            // Needs to be done from inside Python in a case if environment is only modified there.
            let os_module = vm.import("os", 0)?;
            let environ = os_module.get_attr("environ", vm)?;

            let cafile = self.get_env(&environ, CERT_FILE_ENV, vm)?;
            let capath = self.get_env(&environ, CERT_DIR_ENV, vm)?;

            if cafile.is_some() || capath.is_some() {
                // Load certificates and certificate revocation lists from specified paths.
                let has_cafile = cafile.is_some();
                let args = LoadVerifyLocationsArgs {
                    cafile,
                    capath: if has_cafile { None } else { capath },
                    cadata: OptionalArg::Missing,
                };
                self.load_verify_locations(args, vm)?;
            } else {
                // Enable system verifier only if we do not have env vars set.
                self.use_system_certificates.store(true, Ordering::Relaxed);
            }

            Ok(())
        }

        fn get_env(
            &self,
            environ: &PyObjectRef,
            name: &str,
            vm: &VirtualMachine,
        ) -> PyResult<Option<FsPath>> {
            let res = environ.get_item(name, vm);
            match res {
                Ok(obj) => FsPath::try_from_object(vm, obj).map(Some),
                Err(err) if err.fast_isinstance(vm.ctx.exceptions.key_error) => Ok(None),
                Err(err) => Err(err),
            }
        }

        #[pymethod]
        fn _set_alpn_protocols(&self, protos: ArgBytesLike, vm: &VirtualMachine) -> PyResult<()> {
            use std::io::Read;

            let bytes = protos.borrow_buf();
            let mut bytes: &[u8] = &bytes;

            let mut alpn_protocols = Vec::new();
            while !bytes.is_empty() {
                let mut len = 0;
                bytes
                    .read_exact(slice::from_mut(&mut len))
                    .expect("BUG: Impossible");

                if len == 0 {
                    return Err(vm.new_value_error(
                        "Invalid ALPN protocol data: protocol length cannot be 0",
                    ));
                }

                let mut protocol = vec![0; len.into()];
                bytes.read_exact(&mut protocol).map_err(|_| {
                    vm.new_value_error(
                        "Invalid ALPN protocol data: not enough bytes to read protocol",
                    )
                })?;

                alpn_protocols.push(protocol);
            }

            *self.alpn_protocols.write() = alpn_protocols;
            Ok(())
        }

        #[pymethod]
        fn cert_store_stats(&self, vm: &VirtualMachine) -> PyResult {
            vm.with_serde(|s| self.stats.cert_store.serialize(s))
        }

        #[pymethod]
        fn session_stats(&self, vm: &VirtualMachine) -> PyResult {
            vm.with_serde(|s| self.stats.session.serialize(s))
        }

        #[pygetset(setter)]
        fn set_sni_callback(&self, callback: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            if !vm.is_none(&callback) && !callback.is_callable() {
                return Err(vm.new_type_error("sni_callback must be callable or None"));
            }
            *self.sni_callback.write() = callback;
            Ok(())
        }

        #[pygetset]
        fn sni_callback(&self) -> PyObjectRef {
            self.sni_callback.read().clone()
        }

        #[pygetset]
        fn security_level(&self) -> usize {
            let min_bits = self
                .ciphers
                .read()
                .0
                .iter()
                .map(|c| u16::from(c.suite()))
                .map(|i| CIPHER_MAPPINGS.id_to_bits[&i])
                .min()
                .expect("BUG: Impossible");
            for (level, required_bits) in SECURITY_LEVEL_TO_MIN_BITS.iter().enumerate().rev() {
                if min_bits >= *required_bits {
                    return level;
                }
            }
            unreachable!("BUG: Impossible")
        }

        #[pymethod]
        fn load_cert_chain(&self, args: LoadCertChainArgs, vm: &VirtualMachine) -> PyResult<()> {
            let mut password = Password::new(args.password, vm)?;

            let mut priv_key = if let Some(keyfile) = args.keyfile {
                let keyfile_path = keyfile.to_path_buf(vm)?;
                let keyfile_str = keyfile.to_string_lossy();
                let ders = load_der_bytes_from_pem_or_der_file(
                    keyfile_path,
                    &[DerKind::Key],
                    &mut password,
                    vm,
                )
                .map_err(|e| e.into_py_err(vm))?;
                let der =
                    ensure_single_der_bytes(&keyfile_str, ders).map_err(|e| e.into_py_err(vm))?;
                Some(der.bytes)
            } else {
                None
            };

            let kinds = if priv_key.is_some() {
                &[DerKind::Cert][..]
            } else {
                &[DerKind::Cert, DerKind::Key][..]
            };

            let certfile_path = args.certfile.to_path_buf(vm)?;
            let ders = load_der_bytes_from_pem_or_der_file(certfile_path, kinds, &mut password, vm)
                .map_err(|e| e.into_py_err(vm))?;
            let mut certs = Vec::with_capacity(ders.len());
            for der in ders {
                if der.kind == DerKind::Cert {
                    certs.push(der.bytes);
                } else {
                    // Private key
                    if priv_key.is_some() {
                        return Err(vm.new_value_error("more than one private key found"));
                    }
                    priv_key = Some(der.bytes);
                }
            }

            let priv_key =
                priv_key.ok_or_else(|| SslError::Ssl("PEM lib".to_string()).into_py_err(vm))?;

            // Check that certificate matches the private key (if any).
            let first = certs
                .first()
                .ok_or_else(|| SslError::Ssl("PEM lib".to_string()).into_py_err(vm))?;
            let (_, first) = parse_x509_certificate(first).map_err(|e| {
                vm.new_value_error(format!("failed to parse first certificate from chain: {e}"))
            })?;

            // Try to get public key.
            let private_key_der: PrivateKeyDer<'_> = priv_key
                .as_slice()
                .try_into()
                .map_err(|e| vm.new_value_error(format!("failed to parse private key: {e}")))?;
            let sign_key = CryptoExt::get_ext()
                .any_supported_key(&private_key_der)
                .map_err(|e| vm.new_value_error(format!("failed to parse private key: {e}")))?;
            let pub_key = sign_key
                .public_key()
                .ok_or_else(|| vm.new_value_error("can not get public key"))?;

            if first.tbs_certificate.public_key().raw != pub_key.as_ref() {
                return Err(SslError::Ssl("KEY_VALUES_MISMATCH".to_string()).into_py_err(vm));
            }

            // Check remaining certificates.
            for cert in &certs[1..] {
                let _ = parse_x509_certificate(cert).map_err(|e| {
                    vm.new_value_error(format!("failed to parse certificate from chain: {e}"))
                })?;
            }

            self.cert_chain.write().push((
                certs.into_iter().map(Into::into).collect(),
                priv_key.try_into().map_err(|e| {
                    vm.new_value_error(format!("failed to prepare private key: {e}"))
                })?,
            ));
            Ok(())
        }

        #[pymethod]
        fn load_verify_locations(
            &self,
            args: LoadVerifyLocationsArgs,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let cafile = args.cafile;
            let capath = args.capath;
            let cadata = args.cadata.flatten();

            if cafile.is_none() && capath.is_none() && cadata.is_none() {
                return Err(vm.new_type_error("cafile, capath and cadata cannot be all omitted"));
            }

            // Load from cafile
            if let Some(cafile) = cafile {
                let ders = load_der_bytes_from_pem_or_der_file(
                    cafile.to_path_buf(vm)?,
                    &[DerKind::Cert, DerKind::Crl],
                    &mut Password::None,
                    vm,
                )
                .map_err(|e| e.into_py_err(vm))?;
                self.cert_store.write().add_ders(&ders);
            }

            // Load from capath
            if let Some(capath) = capath {
                let capath = capath.to_path_buf(vm)?;
                let paths = vm
                    .allow_threads(|| rustpython_host_env::fs::read_dir(capath))
                    .map_err(|e| e.into_pyexception(vm))?;
                for path in paths {
                    let path = path.map_err(|e| e.into_pyexception(vm))?;
                    if !path.path().is_file() {
                        continue;
                    }

                    let ders = load_der_bytes_from_pem_or_der_file(
                        path.path(),
                        &[DerKind::Cert, DerKind::Crl],
                        &mut Password::None,
                        vm,
                    )
                    .map_err(|e| e.into_py_err(vm))?;
                    self.cert_store.write().add_ders(&ders);
                }
            }

            // Load from cadata
            if let Some(cadata) = cadata {
                let (bytes, is_pem_text) = match cadata {
                    Either::A(d) => (d.as_bytes().to_vec(), true),
                    Either::B(d) => (d.borrow_buf().to_vec(), false),
                };

                let ders = if is_pem_text {
                    let (ders, _) = load_der_bytes_from_pem(
                        "<cadata>",
                        &bytes,
                        &[DerKind::Cert, DerKind::Crl],
                        &mut Password::None,
                        vm,
                    )
                    .map_err(|e| e.into_py_err(vm))?;

                    if ders.is_empty() && !bytes.is_empty() {
                        return Err(SslError::CadataNoStartLine.into_py_err(vm));
                    }

                    ders
                } else {
                    load_der_bytes_from_der(
                        "<cadata>",
                        &bytes,
                        &[DerKind::Cert, DerKind::Crl],
                        &mut Password::None,
                        vm,
                    )
                    .map_err(|_| SslError::CadataNotEnoughData.into_py_err(vm))?
                };

                self.cert_store.write().add_ders(&ders);
            }

            Ok(())
        }

        #[pymethod]
        fn get_ca_certs(&self, args: GetCertArgs, vm: &VirtualMachine) -> PyResult<PyListRef> {
            let binary_form = if let OptionalArg::Present(binary_form) = args.binary_form {
                binary_form
            } else {
                false
            };

            let cert_store = self.cert_store.read();
            let mut list = Vec::<PyObjectRef>::with_capacity(cert_store.all_certs().len());
            if binary_form {
                for cert in cert_store.all_certs() {
                    list.push(vm.ctx.new_bytes(cert.clone()).into());
                }
            } else {
                for cert in cert_store.all_certs() {
                    list.push(CertInfo::parse_to_py(cert, vm)?);
                }
            }
            Ok(vm.ctx.new_list(list))
        }

        #[pymethod]
        fn _wrap_socket(
            zelf: PyRef<Self>,
            args: WrapSocketArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<PySSLSocket>> {
            let io = Io::from_socket(args.sock, vm)?;
            Self::create_socket(
                zelf,
                io,
                args.server_side,
                args.server_hostname,
                args.owner,
                args.session,
                vm,
            )
        }

        #[pymethod]
        fn _wrap_bio(
            zelf: PyRef<Self>,
            args: WrapBioArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<PySSLSocket>> {
            let io = Io::from_bio(args.incoming, args.outgoing);
            Self::create_socket(
                zelf,
                io,
                args.server_side,
                args.server_hostname,
                args.owner,
                args.session,
                vm,
            )
        }

        fn create_socket(
            zelf: PyRef<Self>,
            io: Io,
            server_side: OptionalArg<bool>,
            server_hostname: OptionalArg<Option<PyUtf8StrRef>>,
            owner: PyObjectRef,
            session: OptionalArg<Option<PyObjectRef>>,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<PySSLSocket>> {
            let server_side = server_side.unwrap_or(false);
            let server_hostname = server_hostname
                .into_option()
                .flatten()
                .map(|h| h.to_string());
            let owner = owner.downgrade(None, vm)?;

            if server_side && zelf.protocol == PROTOCOL_TLS_CLIENT {
                return Err(SslError::Ssl(
                    "Cannot create a server socket with a PROTOCOL_TLS_CLIENT context".to_string(),
                )
                .into_py_err(vm));
            }
            if !server_side && zelf.protocol == PROTOCOL_TLS_SERVER {
                return Err(SslError::Ssl(
                    "Cannot create a client socket with a PROTOCOL_TLS_SERVER context".to_string(),
                )
                .into_py_err(vm));
            }

            let state = if server_side {
                State::new_handshaking_server()
            } else {
                if server_hostname.as_ref().is_some_and(|h| h.contains('\0')) {
                    return Err(vm.new_type_error("server_hostname cannot contain null bytes"));
                }
                let server_hostname = server_hostname
                    .as_ref()
                    .map(|h| ServerName::try_from(h.as_str()))
                    .transpose()
                    .map_err(|e| vm.new_value_error(format!("Invalid server name: {e}")))?
                    .map(|h| h.to_owned());
                State::new_handshaking_client(Self::create_client_connection(
                    &zelf,
                    server_hostname,
                    vm,
                )?)
            };

            let socket = PySSLSocket {
                context: PyRwLock::new(zelf),
                owner,
                io: PyRwLock::new(io),
                server_side,
                server_hostname: PyRwLock::new(server_hostname),
                state: PyRwLock::new(state),
                shared_ciphers: PyRwLock::new(None),
            };

            // TODO: Implement session support.
            if let Some(session) = session.into_option().flatten() {
                socket.set_session(session, vm)?;
            }

            socket
                .into_ref_with_type(vm, vm.class("_ssl", "_SSLSocket"))
                .map_err(|_| vm.new_type_error("Failed to create SSLSocket"))
        }

        fn create_client_connection(
            &self,
            server_name: Option<ServerName<'static>>,
            vm: &VirtualMachine,
        ) -> PyResult<Connection> {
            let crypto = self.create_crypto_provider();

            // Certificate verifier.
            let use_system_certificates = self.use_system_certificates.load(Ordering::Relaxed);
            let crl_check = CrlCheck::from_verify_flags(self.verify_flags());
            if use_system_certificates && !matches!(crl_check, CrlCheck::None) {
                _warnings::warn(
                    vm.ctx.exceptions.runtime_warning,
                    "rustls default platform verifier does not support disabling ssl.VERIFY_CRL_CHECK_*".to_owned(),
                    2,
                    vm,
                )?;
            };

            let verifier = CustomServerCertVerifier::new(
                self.verify_mode() != CERT_NONE,
                use_system_certificates,
                &self.cert_store.read(),
                crypto.clone(),
                self.check_hostname(),
                crl_check,
            )
            .map_err(|e| e.into_py_err(vm))?;

            // Client configuration.
            let config = ClientConfig::builder_with_provider(crypto)
                .with_protocol_versions(&self.get_supported_versions())
                .map_err(|e| vm.new_value_error(format!("failed to create rustls client: {e}")))?
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(verifier));
            let mut config = if let Some((cert_chain, priv_key)) = self.cert_chain.read().last() {
                config
                    .with_client_auth_cert(cert_chain.clone(), priv_key.clone_key())
                    .map_err(|e| {
                        vm.new_value_error(format!("failed to set client certificate chain: {e}"))
                    })?
            } else {
                config.with_no_client_auth()
            };

            config.alpn_protocols = self.alpn_protocols.read().clone();
            config.resumption = self.session_cache.clone();

            // Server name.
            let server_name = if let Some(server_name) = server_name {
                server_name
            } else {
                config.enable_sni = false;
                // rustls always needs a ServerName, so provide it an invalid IPv4 address.
                ServerName::IpAddress(IpAddr::V4(Ipv4Addr::from([0, 0, 0, 0])))
            };

            Ok(ClientConnection::new(Arc::new(config), server_name)
                .map_err(|e| SslError::from_rustls(e).into_py_err(vm))?
                .into())
        }

        // PySSLSocket::do_handshake() calls this after receiving ClientHello with Listener.
        fn create_server_connection(
            &self,
            accepted: Accepted,
            vm: &VirtualMachine,
        ) -> PyResult<Result<Connection, (rustls::Error, AcceptedAlert)>> {
            let crypto = self.create_crypto_provider();

            // Find certificate chain with a key matching algorithms requested by client.
            // TODO: Search by a requested host name too.
            let cert_chain = self.cert_chain.read();
            if cert_chain.is_empty() {
                return Err(SslError::from_rustls(rustls::Error::PeerIncompatible(
                    rustls::PeerIncompatible::NoCipherSuitesInCommon,
                ))
                .into_py_err(vm));
            }
            let (cert_chain, priv_key) = {
                let client_hello = accepted.client_hello();
                let signature_schemes = client_hello.signature_schemes();
                cert_chain
                    .iter()
                    .find(|(cert_chain, priv_key)| {
                        CertifiedKey::from_der(cert_chain.clone(), priv_key.clone_key(), &crypto)
                            .is_ok_and(|certified_key| {
                                certified_key.key.choose_scheme(signature_schemes).is_some()
                            })
                    })
                    .unwrap_or_else(|| cert_chain.last().expect("BUG: Impossible"))
            };

            // Server configuration.
            let mut config = ServerConfig::builder_with_provider(crypto.clone())
                .with_protocol_versions(&self.get_supported_versions())
                .map_err(|e| vm.new_value_error(format!("failed to create rustls server: {e}")))?
                .with_client_cert_verifier(self.create_client_cert_verifier(crypto, vm)?)
                .with_single_cert(cert_chain.clone(), priv_key.clone_key())
                .map_err(|e| {
                    vm.new_value_error(format!("failed to set server certificate chain: {e}"))
                })?;

            // ALPN protocols.
            let alpn_protocols = self.alpn_protocols.read();
            if accepted
                .client_hello()
                .alpn()
                .is_some_and(|mut client_protocols| {
                    client_protocols.any(|client_protocol| {
                        alpn_protocols
                            .iter()
                            .any(|server_protocol| server_protocol.as_slice() == client_protocol)
                    })
                })
            {
                // Configure acceptable ALPN protocols only if client's is supported one.
                // This matches cpython's ssl behaviour that allows connections when server
                // does not know protocol requested by client.
                config.alpn_protocols = alpn_protocols.clone();
            }

            config.ignore_client_order =
                self.is_one_of_options_enabled(OP_CIPHER_SERVER_PREFERENCE);

            if self.is_one_of_options_enabled(OP_NO_TICKET) || (self.num_tickets() == 0) {
                config.send_tls13_tickets = 0;
            } else {
                config.ticketer = (CryptoExt::get_ext().ticketer)().map_err(|e| {
                    vm.new_value_error(format!("failed to create TLS ticketer: {e}"))
                })?;
                config.send_tls13_tickets = self.num_tickets();
            }

            Ok(match accepted.into_connection(Arc::new(config)) {
                Ok(conn) => Ok(conn.into()),
                Err((err, alert)) => Err((err, alert)),
            })
        }

        fn create_crypto_provider(&self) -> Arc<CryptoProvider> {
            let mut provider = CryptoExt::get_provider().clone();

            let suite_b = {
                let ciphers = self.ciphers.read();
                provider.cipher_suites = ciphers.0.clone();
                if let Some(kx_groups) = ciphers.1.as_ref() {
                    provider.kx_groups = kx_groups.clone();
                    true
                } else {
                    false
                }
            };

            {
                let ecdh_curve = self.ecdh_curve.read();
                if !suite_b && let Some(ecdh_curve) = ecdh_curve.as_ref() {
                    provider.kx_groups = ecdh_curve.clone();
                }
            }

            Arc::new(provider)
        }

        fn get_supported_versions(&self) -> Vec<&'static SupportedProtocolVersion> {
            let mut versions =
                Vec::<&'static SupportedProtocolVersion>::with_capacity(ALL_VERSIONS.len());
            for version in ALL_VERSIONS {
                let proto = u16::from(version.version).into();
                let add = match version.version {
                    ProtocolVersion::TLSv1_2 => {
                        self.proto_within_range(proto)
                            && !self.is_one_of_options_enabled(OP_NO_TLSv1_2)
                    }

                    ProtocolVersion::TLSv1_3 => {
                        self.proto_within_range(proto)
                            && !self.is_one_of_options_enabled(OP_NO_TLSv1_3)
                    }

                    _ => self.proto_within_range(proto),
                };
                if add {
                    versions.push(version);
                }
            }
            versions
        }

        fn proto_within_range(&self, proto: i32) -> bool {
            (self.minimum_version()..=self.maximum_version()).contains(&proto)
        }

        fn create_client_cert_verifier(
            &self,
            crypto: Arc<CryptoProvider>,
            vm: &VirtualMachine,
        ) -> PyResult<Arc<dyn ClientCertVerifier>> {
            let verify_mode = self.verify_mode();
            if verify_mode == CERT_NONE {
                Ok(Arc::new(NoClientAuth))
            } else {
                let cert_store = self.cert_store.read();
                let builder = WebPkiClientVerifier::builder_with_provider(
                    Arc::new(cert_store.certs.clone()),
                    crypto,
                )
                .with_crls(cert_store.crls.clone());
                if verify_mode == CERT_OPTIONAL {
                    builder.allow_unauthenticated().build()
                } else {
                    builder.build()
                }
                .map_err(|e| {
                    SslError::Ssl(format!("failed to create client certificate verifier: {e}"))
                        .into_py_err(vm)
                })
            }
        }

        fn is_one_of_options_enabled(&self, op: i32) -> bool {
            (self.options() & op) != 0
        }

        #[pygetset(setter)]
        fn set_verify_flags(&self, value: i32) {
            self.verify_flags.store(value, Ordering::Relaxed);
        }

        #[pygetset]
        fn verify_flags(&self) -> i32 {
            self.verify_flags.load(Ordering::Relaxed)
        }

        // Completely unsupported by rustls.

        #[pymethod]
        fn load_dh_params(&self, _filepath: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            Err(SslError::PemLib(
                "NO_START_LINE: ssl.SSLContext.load_dh_params is not supported by rustls"
                    .to_string(),
            )
            .into_py_err(vm))
        }

        #[pygetset]
        fn post_handshake_auth(&self) -> bool {
            self.post_handshake_auth.load(Ordering::Relaxed)
        }

        #[pygetset(setter)]
        fn set_post_handshake_auth(&self, value: bool, vm: &VirtualMachine) -> PyResult<()> {
            // Some libraries, like urllib.request, always set this to True for whatever reason.
            self.post_handshake_auth.store(value, Ordering::Relaxed);
            if value {
                _warnings::warn(
                    vm.ctx.exceptions.deprecation_warning,
                    "ssl.SSLContext.post_handshake_auth is not supported by rustls".to_string(),
                    2,
                    vm,
                )?;
            }
            Ok(())
        }

        #[pygetset(setter)]
        fn set__msg_callback(&self, callback: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            let is_none = vm.is_none(&callback);
            if !is_none && !callback.is_callable() {
                return Err(vm.new_type_error("msg_callback must be callable or None"));
            }

            if !is_none {
                _warnings::warn(
                    vm.ctx.exceptions.deprecation_warning,
                    "rustls does not support SSLContext._msg_callback".to_string(),
                    2,
                    vm,
                )?;
            }

            *self.msg_callback.write() = callback;
            Ok(())
        }

        #[pygetset]
        fn _msg_callback(&self) -> PyObjectRef {
            self.msg_callback.read().clone()
        }

        #[pygetset(setter)]
        fn set__host_flags(&self, value: i32) {
            self.host_flags.store(value, Ordering::Relaxed);
        }

        #[pygetset]
        fn _host_flags(&self) -> i32 {
            self.host_flags.load(Ordering::Relaxed)
        }
    }

    #[derive(FromArgs)]
    struct LoadCertChainArgs {
        certfile: FsPath,

        #[pyarg(any, optional)]
        keyfile: Option<FsPath>,

        #[pyarg(any, optional)]
        password: OptionalArg<PyObjectRef>,
    }

    #[derive(FromArgs)]
    struct LoadVerifyLocationsArgs {
        #[pyarg(any, default)]
        cafile: Option<FsPath>,

        #[pyarg(any, default)]
        capath: Option<FsPath>,

        #[pyarg(any, optional, error_msg = "cadata should be a str or bytes")]
        cadata: OptionalArg<Option<Either<PyUtf8StrRef, ArgBytesLike>>>,
    }

    #[derive(FromArgs)]
    struct GetCertArgs {
        #[pyarg(any, optional)]
        binary_form: OptionalArg<bool>,
    }

    #[derive(FromArgs)]
    struct WrapSocketArgs {
        sock: PyObjectRef,
        #[pyarg(positional, optional)]
        server_side: OptionalArg<bool>,
        #[pyarg(positional, optional)]
        server_hostname: OptionalArg<Option<PyUtf8StrRef>>,
        #[pyarg(named)]
        owner: PyObjectRef,
        #[pyarg(named, optional)]
        session: OptionalArg<Option<PyObjectRef>>,
    }

    #[derive(FromArgs)]
    struct WrapBioArgs {
        incoming: PyObjectRef,
        outgoing: PyObjectRef,
        #[pyarg(named, optional)]
        server_side: OptionalArg<bool>,
        #[pyarg(named, optional)]
        server_hostname: OptionalArg<Option<PyUtf8StrRef>>,
        #[pyarg(named)]
        owner: PyObjectRef,
        #[pyarg(named, optional)]
        session: OptionalArg<Option<PyObjectRef>>,
    }

    // SSLSocket - represents a TLS-wrapped socket
    #[pyattr]
    #[pyclass(module = "_ssl", name = "_SSLSocket")]
    #[derive(Debug, PyPayload)]
    struct PySSLSocket {
        context: PyRwLock<PyRef<PySSLContext>>,
        owner: PyRef<PyWeak>,
        io: PyRwLock<Io>,
        server_side: bool,
        server_hostname: PyRwLock<Option<String>>,
        state: PyRwLock<State>,
        shared_ciphers: PyRwLock<Option<Vec<SupportedCipherSuite>>>,
    }

    impl Representable for PySSLSocket {
        #[inline]
        fn repr_str(_zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            Ok("<SSLSocket>".to_owned())
        }
    }

    impl Constructor for PySSLSocket {
        type Args = ();

        fn slot_new(_cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            Err(vm.new_type_error(
                "Cannot directly instantiate SSLSocket, use SSLContext.wrap_socket()",
            ))
        }

        fn py_new(_cls: &Py<PyType>, _args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            Err(vm.new_not_implemented_error(
                "Cannot directly instantiate SSLSocket, use SSLContext.wrap_socket()",
            ))
        }
    }

    #[pyclass(with(Constructor, Representable), flags(BASETYPE))]
    impl PySSLSocket {
        #[pygetset(setter)]
        fn set_context(&self, value: PyRef<PySSLContext>, _vm: &VirtualMachine) {
            *self.context.write() = value;
        }

        #[pygetset]
        fn context(&self) -> PyRef<PySSLContext> {
            self.context.read().clone()
        }

        #[pygetset]
        fn server_side(&self) -> bool {
            self.server_side
        }

        #[pygetset(setter)]
        fn set_server_hostname(&self, value: Option<PyUtf8StrRef>) {
            *self.server_hostname.write() = value.map(|s| s.to_string());
        }

        #[pygetset]
        fn server_hostname(&self) -> Option<String> {
            self.server_hostname.read().clone()
        }

        #[pymethod]
        fn do_handshake(&self, vm: &VirtualMachine) -> PyResult<()> {
            'outer: loop {
                let mut state = self.state.write();

                match &mut *state {
                    State::ServerWaitingForClientHello(acceptor) => {
                        let accepted = loop {
                            {
                                let mut io = self.io.write();
                                let _ = io.with_io(vm, |io| acceptor.read_tls(io))?;
                            }

                            match acceptor.accept() {
                                Ok(Some(accepted)) => break accepted,
                                Ok(None) => {}

                                Err((err, alert)) => {
                                    *state = State::new_alert_from_rustls_error(err, alert, vm)?;
                                    continue 'outer;
                                }
                            }
                        };

                        let context = self.context();
                        let hello = accepted.client_hello();

                        // Remember shared cipher suites.
                        {
                            let our_ciphers = context.ciphers.read();
                            *self.shared_ciphers.write() = Some(
                                hello
                                    .cipher_suites()
                                    .iter()
                                    .filter_map(|c| {
                                        our_ciphers
                                            .0
                                            .iter()
                                            .find(|oc| u16::from(*c) == u16::from(oc.suite()))
                                    })
                                    .copied()
                                    .collect(),
                            );
                        }

                        // Call SNI callback (if any).
                        let sni_callback = context.sni_callback();
                        if !vm.is_none(&sni_callback) {
                            let owner = self.owner.upgrade().ok_or_else(|| {
                                vm.new_value_error(
                                    "ssl.SSLSocket was dropped before _ssl._SSLSocket",
                                )
                            })?;
                            let res = sni_callback.call((owner, hello.server_name(), context), vm);

                            match res {
                                Ok(res) if vm.is_none(&res) => {}

                                Ok(res) => match res.try_to_value::<i32>(vm) {
                                    Ok(alert_code)
                                        if (0..=u8::MAX as i32).contains(&alert_code) =>
                                    {
                                        let error = SslError::Ssl(
                                            "TLS connection rejected by SNI callback".to_string(),
                                        )
                                        .into_py_err(vm);

                                        *state = State::new_alert_from_sni_callback_error(
                                            error,
                                            alert_code as u8,
                                        );
                                        continue 'outer;
                                    }

                                    _ => {
                                        let type_error = vm.new_type_error(format!(
                                            "servername callback must return None or an integer, not '{}'",
                                            res.class().name()
                                        ));
                                        vm.run_unraisable(type_error, None, res);
                                        let error = SslError::Ssl(
                                            "SNI callback returned invalid value".to_string(),
                                        )
                                        .into_py_err(vm);

                                        *state = State::new_alert_from_sni_callback_error(
                                            error,
                                            ALERT_DESCRIPTION_INTERNAL_ERROR as u8,
                                        );
                                        continue 'outer;
                                    }
                                },

                                Err(exc) => {
                                    vm.run_unraisable(exc, None, vm.ctx.none());
                                    let error = SslError::Ssl(
                                        "SNI callback raised an exception".to_string(),
                                    )
                                    .into_py_err(vm);
                                    *state = State::new_alert_from_sni_callback_error(
                                        error,
                                        ALERT_DESCRIPTION_HANDSHAKE_FAILURE as u8,
                                    );
                                    continue 'outer;
                                }
                            }
                        };

                        // Create rustls connection.
                        let conn =
                            match self.context.read().create_server_connection(accepted, vm)? {
                                Ok(conn) => conn,
                                Err((err, alert)) => {
                                    *state = State::new_alert_from_rustls_error(err, alert, vm)?;
                                    continue;
                                }
                            };
                        *state = State::HasConnection {
                            state: ConnectionState::Handshaking,
                            conn,
                        };
                        let State::HasConnection { state, conn, .. } = &mut *state else {
                            unreachable!("BUG: Impossible")
                        };

                        self.complete_io(conn, true, vm)?;
                        *state = ConnectionState::Connected(CloseNotifyState::None);
                        break Ok(());
                    }

                    State::ServerSendingAlert {
                        error,
                        alert_buf,
                        alert_buf_pos,
                    } => {
                        let mut io = self.io.write();
                        let sent = io.with_io(vm, |io| io.write(&alert_buf[*alert_buf_pos..]))?;
                        *alert_buf_pos += sent;
                        if *alert_buf_pos == alert_buf.len() {
                            break Err(error.clone());
                        }
                    }

                    State::HasConnection {
                        state: conn_state @ ConnectionState::Handshaking,
                        conn,
                    } => {
                        self.complete_io(conn, true, vm)?;
                        *conn_state = ConnectionState::Connected(CloseNotifyState::None);
                        break Ok(());
                    }

                    State::HasConnection {
                        state:
                            ConnectionState::Connected(_)
                            | ConnectionState::ShuttingDown
                            | ConnectionState::ShutDown,
                        ..
                    } => break Ok(()), // handshake already done
                };
            }
        }

        #[pymethod]
        fn read(
            &self,
            len: isize,
            buffer: OptionalArg<ArgMemoryBuffer>,
            vm: &VirtualMachine,
        ) -> PyResult {
            // Ensure handshake done.
            self.do_handshake(vm)?;

            // Prepare buffer.
            let mut owned_buffer = buffer
                .map(Either::A)
                .unwrap_or_else(|| Either::B(Vec::new()));

            let read = {
                let buffer_mut = match &mut owned_buffer {
                    Either::A(buffer) if len < 0 => &mut buffer.borrow_buf_mut()[..],

                    Either::A(buffer) => {
                        let len = buffer.len().min(len as usize);
                        &mut buffer.borrow_buf_mut()[..len]
                    }

                    Either::B(_) if len < 0 => return Err(vm.new_value_error("negative read len")),

                    Either::B(buffer) => {
                        let len = len as usize;
                        buffer.resize(len, 0);
                        &mut buffer[..]
                    }
                };

                let mut state = self.state.write();
                self.read_inner(&mut state, buffer_mut, vm)?
            };

            if let Some(read) = read {
                match owned_buffer {
                    Either::A(_) => Ok(vm.ctx.new_int(read).into()),

                    Either::B(mut bytes) => {
                        bytes.truncate(read);
                        Ok(vm.ctx.new_bytes(bytes).into())
                    }
                }
            } else {
                // Close Notify already received.
                match owned_buffer {
                    Either::A(_) => Ok(vm.ctx.new_int(0).into()),
                    Either::B(_) => Ok(vm.ctx.new_bytes(Vec::new()).into()),
                }
            }
        }

        fn read_inner(
            &self,
            state: &mut State,
            buffer: &mut [u8],
            vm: &VirtualMachine,
        ) -> PyResult<Option<usize>> {
            let (conn, conn_state) = match &mut *state {
                State::ServerWaitingForClientHello(_)
                | State::ServerSendingAlert { .. }
                | State::HasConnection {
                    state: ConnectionState::Handshaking,
                    ..
                } => {
                    unreachable!("BUG: read() is in wrong state")
                }

                State::HasConnection {
                    state:
                        conn_state @ ConnectionState::Connected(
                            CloseNotifyState::None | CloseNotifyState::Sent,
                        ),
                    conn,
                } => (conn, conn_state),

                State::HasConnection {
                    state: ConnectionState::Connected(CloseNotifyState::Received),
                    ..
                } => {
                    return Ok(None);
                }

                State::HasConnection {
                    state: ConnectionState::ShuttingDown | ConnectionState::ShutDown,
                    ..
                } => {
                    return Err(SslError::ZeroReturn.into_py_err(vm));
                }
            };

            // Do the read.
            loop {
                match conn.reader().read(buffer) {
                    Ok(read) => {
                        if (read == 0) && !buffer.is_empty() {
                            // Close Notify received.
                            match conn_state {
                                ConnectionState::Connected(CloseNotifyState::None) => {
                                    *conn_state =
                                        ConnectionState::Connected(CloseNotifyState::Received)
                                }

                                ConnectionState::Connected(CloseNotifyState::Sent) => {
                                    // Sent + Received => shutdown almost complete, need to ensure that IO is done.
                                    *conn_state = ConnectionState::ShuttingDown
                                }

                                _ => {
                                    unreachable!(
                                        "BUG: Other ConnectionState variants handled earlier in read()"
                                    )
                                }
                            }
                        }
                        break Ok(Some(read));
                    }

                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        // There is no plaintext data in internal buffers, need to do IO.
                        self.complete_io(conn, true, vm)?;
                    }

                    Err(err) => return Err(SslError::Io(err).into_py_err(vm)),
                }
            }
        }

        #[pymethod]
        fn write(&self, data: ArgBytesLike, vm: &VirtualMachine) -> PyResult<usize> {
            // Ensure handshake done.
            self.do_handshake(vm)?;

            let mut state = self.state.write();
            let conn = match &mut *state {
                State::ServerWaitingForClientHello(_)
                | State::ServerSendingAlert { .. }
                | State::HasConnection {
                    state: ConnectionState::Handshaking,
                    ..
                } => {
                    unreachable!("BUG: write() is in wrong state")
                }

                State::HasConnection {
                    state:
                        ConnectionState::Connected(CloseNotifyState::None | CloseNotifyState::Received),
                    conn,
                } => conn,

                State::HasConnection {
                    state:
                        ConnectionState::Connected(CloseNotifyState::Sent)
                        | ConnectionState::ShuttingDown
                        | ConnectionState::ShutDown,
                    ..
                } => {
                    return Err(SslError::ZeroReturn.into_py_err(vm));
                }
            };

            // Send previously queued data, if any.
            self.complete_io(conn, false, vm)?;

            let data = data.borrow_buf();
            let written = conn
                .writer()
                .write(&data)
                .map_err(|e| SslError::Io(e).into_py_err(vm))?;

            self.complete_io(conn, false, vm)?;

            Ok(written)
        }

        #[pymethod]
        fn shutdown(&self, vm: &VirtualMachine) -> PyResult {
            loop {
                let mut state = self.state.write();

                match &mut *state {
                    State::ServerWaitingForClientHello(_)
                    | State::ServerSendingAlert { .. }
                    | State::HasConnection {
                        state: ConnectionState::Handshaking,
                        ..
                    } => {
                        return Err(SslError::Ssl(
                            "cannot perform TLS shutdown before handshake completed".to_string(),
                        )
                        .into_py_err(vm));
                    }

                    State::HasConnection {
                        state:
                            ConnectionState::Connected(close_notify_state @ CloseNotifyState::None),
                        conn,
                    } => {
                        conn.send_close_notify();
                        *close_notify_state = CloseNotifyState::Sent;
                    }

                    State::HasConnection {
                        state: conn_state @ ConnectionState::Connected(CloseNotifyState::Received),
                        conn,
                    } => {
                        conn.send_close_notify();
                        *conn_state = ConnectionState::ShuttingDown;
                    }

                    State::HasConnection {
                        state: ConnectionState::Connected(CloseNotifyState::Sent),
                        ..
                    } => {
                        let mut byte = 0;
                        if self.read_inner(&mut state, slice::from_mut(&mut byte), vm)? == Some(1) {
                            return Err(SslError::Ssl(format!(
                                "Expected TLS Close Notify but received plaintext byte {byte}"
                            ))
                            .into_py_err(vm));
                        }
                    }

                    State::HasConnection {
                        state: conn_state @ ConnectionState::ShuttingDown,
                        conn,
                    } => {
                        self.complete_io(conn, true, vm)?;
                        *conn_state = ConnectionState::ShutDown;
                        break;
                    }

                    State::HasConnection {
                        state: ConnectionState::ShutDown,
                        ..
                    } => {
                        break;
                    }
                };
            }

            Ok(self.io.read().to_socket(vm))
        }

        // When handshaking, complete_io() returns only after handshake is complete.
        // TODO: This might call certificate verifier which might be blocking and require network access on its own.
        //     option 1: Extract process_new_packets() from complete_io() to wrap it in allow_threads().
        //     option 2: Introduce VirtualMachine::disallow_threads() and use it inside the IO wrapper instead.
        fn complete_io(
            &self,
            conn: &mut Connection,
            read_and_write: bool,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            // complete_io() when writing if !conn.wants_write() may read data past the Close Notify.
            // TODO: Remove this check when proper rustls unbuffered API is used.
            if read_and_write || conn.wants_write() {
                let mut io = self.io.write();
                let _ = io.with_io(vm, |io| conn.complete_io(io))?;
            }
            Ok(())
        }

        #[pymethod]
        fn pending(&self, vm: &VirtualMachine) -> PyResult<usize> {
            self.state
                .write()
                .get_connection_mut()
                .map(|conn| self.pending_inner(conn, vm).map(|l| l.unwrap_or(0)))
                .transpose()
                .map(|l| l.unwrap_or(0))
        }

        fn pending_inner(
            &self,
            conn: &mut Connection,
            vm: &VirtualMachine,
        ) -> PyResult<Option<usize>> {
            match conn.reader().fill_buf().map(|b| b.len()) {
                Ok(len) => Ok(Some(len)),
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
                Err(err) => Err(SslError::Io(err).into_py_err(vm)),
            }
        }

        #[pymethod]
        fn getpeercert(
            &self,
            args: GetPeerCertArgs,
            vm: &VirtualMachine,
        ) -> PyResult<Option<PyObjectRef>> {
            let state = self.state.read();
            let Some(conn) = state.get_connection() else {
                return Err(vm.new_value_error("handshake not done yet"));
            };

            let binary_form = args.binary_form.unwrap_or(false);

            let Some(certs) = conn.peer_certificates() else {
                return Ok(None);
            };
            let Some(cert) = certs.first() else {
                return Ok(None);
            };

            if binary_form {
                Ok(Some(vm.ctx.new_bytes(cert.as_ref().to_vec()).into_object()))
            } else if self.context.read().verify_mode() == CERT_NONE {
                Ok(Some(vm.ctx.new_dict().into()))
            } else {
                CertInfo::parse_to_py(cert, vm).map(|cert| Some(cert.into_object()))
            }
        }

        #[pymethod]
        fn cipher(&self, vm: &VirtualMachine) -> Option<PyTupleRef> {
            self.state
                .read()
                .get_connection()
                .and_then(|c| c.negotiated_cipher_suite())
                .map(|c| cipher_to_tuple(&c, vm))
        }

        #[pymethod]
        fn version(&self) -> Option<&'static str> {
            self.state
                .read()
                .get_connection()
                .and_then(|c| c.negotiated_cipher_suite())
                .map(|c| cipher_to_version(&c))
        }

        #[pymethod]
        fn selected_alpn_protocol(&self, vm: &VirtualMachine) -> PyResult<Option<String>> {
            self.state
                .read()
                .get_connection()
                .and_then(|conn| conn.alpn_protocol())
                .map(|a| {
                    String::from_utf8(a.to_vec())
                        .map_err(|_| vm.new_value_error("ALPN protocol is not valid UTF-8"))
                })
                .transpose()
        }

        #[pymethod]
        fn session_reused(&self) -> bool {
            self.state
                .read()
                .get_connection()
                .is_some_and(|c| matches!(c.handshake_kind(), Some(HandshakeKind::Resumed)))
        }

        #[pymethod]
        fn get_verified_chain(&self, vm: &VirtualMachine) -> Option<PyObjectRef> {
            // rustls does not expose a separate verified chain.
            self.get_unverified_chain(vm)
        }

        #[pymethod]
        fn get_unverified_chain(&self, vm: &VirtualMachine) -> Option<PyObjectRef> {
            let state = self.state.read();
            let certs = state.get_connection().and_then(|c| c.peer_certificates())?;
            let certs = certs
                .iter()
                .map(|cert| {
                    PySSLCertificate {
                        bytes: cert.as_ref().to_vec(),
                    }
                    .into_ref(&vm.ctx)
                    .into_object()
                })
                .collect();
            Some(vm.ctx.new_list(certs).into_object())
        }

        #[pymethod]
        fn shared_ciphers(&self, vm: &VirtualMachine) -> Option<PyListRef> {
            let shared_ciphers = self.shared_ciphers.read();
            shared_ciphers.as_ref().map(|c| {
                vm.ctx
                    .new_list(c.iter().map(|c| cipher_to_tuple(c, vm).into()).collect())
            })
        }

        // Needed for tests.
        #[pygetset]
        fn owner(&self) -> Option<PyObjectRef> {
            self.owner.upgrade()
        }

        // Completely unsupported by rustls.

        #[pygetset(setter)]
        fn set_session(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            // SSLSocket.session exists to persist sessions within a single process.
            // rustls supports process-local sessions but does not expose any session info.
            // This will change in the next version: https://github.com/rustls/rustls/pull/2907
            // Also see:
            // * https://github.com/rustls/rustls/issues/466#issuecomment-1478728279
            // * https://github.com/rustls/rustls/issues/2287
            // TODO: Implement proper SSLSocket.session when new rustls releases.

            if value.try_downcast_ref::<PySSLSession>(vm).is_err() {
                Err(vm.new_value_error("session is not SSLSession"))
            } else {
                Ok(())
            }
        }

        #[pygetset]
        fn session(&self) -> PySSLSession {
            // Return some dummy session object.
            PySSLSession {
                creation_time: SystemTime::now(),
            }
        }

        #[pymethod]
        fn selected_npn_protocol(&self) -> Option<()> {
            // rustls doesn't support NPN, only ALPN
            None
        }

        #[pymethod]
        fn compression(&self) -> Option<()> {
            // rustls doesn't support compression
            None
        }

        #[pymethod]
        fn verify_client_post_handshake(&self, vm: &VirtualMachine) -> PyResult<()> {
            Err(vm.new_not_implemented_error(
                "ssl.SSLSocket.verify_client_post_handshake() is not supported by rustls",
            ))
        }

        #[pymethod]
        fn get_channel_binding(
            &self,
            cb_type: OptionalArg<PyUtf8StrRef>,
            vm: &VirtualMachine,
        ) -> PyResult<Option<PyBytesRef>> {
            let cb_type = cb_type
                .as_ref()
                .map(|cb_type| cb_type.as_str())
                .unwrap_or("tls-unique");

            // rustls does not support `tls-unique` channel binding:
            //  * https://github.com/rustls/rustls/issues/995
            //  * https://github.com/rustls/rustls/issues/1089
            // Some other channel binding types might be implementable with current rustls.
            Err(vm.new_value_error(format!(
                "{cb_type} channel binding type not supported by rustls"
            )))
        }
    }

    #[derive(FromArgs)]
    struct GetPeerCertArgs {
        #[pyarg(any, optional)]
        binary_form: OptionalArg<bool>,
    }

    #[pyattr]
    #[pyclass(module = "_ssl", name = "MemoryBIO")]
    #[derive(Debug, PyPayload)]
    struct PyMemoryBIO {
        // Internal buffer
        buffer: PyMutex<Vec<u8>>,
        // EOF flag
        eof: AtomicBool,
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
                buffer: PyMutex::new(Vec::new()),
                eof: AtomicBool::new(false),
            })
        }
    }

    #[pyclass(with(Constructor), flags(BASETYPE))]
    impl PyMemoryBIO {
        #[pymethod]
        fn read(&self, len: OptionalArg<isize>, vm: &VirtualMachine) -> PyResult<PyBytesRef> {
            let len = len
                .map(|l| l.try_into())
                .transpose()
                .map_err(|_| vm.new_value_error(format!("length is out of range: {len:?}")))?;

            let mut buffer = self.buffer.lock();

            if buffer.is_empty() && self.eof.load(Ordering::Relaxed) {
                // Return empty bytes at EOF
                return Ok(vm.ctx.new_bytes(vec![]));
            }

            let len = len.unwrap_or(buffer.len());
            let len = len.min(buffer.len());
            let data = buffer.drain(..len).collect::<Vec<u8>>();

            Ok(vm.ctx.new_bytes(data))
        }

        #[pymethod]
        fn write(&self, buf: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
            // Check if buf is contiguous if it is a memoryview
            if let Ok(mem_view) = buf.get_attr("c_contiguous", vm) {
                // It's a memoryview, check if contiguous
                let is_contiguous: bool = mem_view.try_to_bool(vm)?;
                if !is_contiguous {
                    return Err(vm.new_exception_msg(
                        vm.ctx.exceptions.buffer_error.to_owned(),
                        "non-contiguous buffer is not supported".into(),
                    ));
                }
            }
            // Convert to bytes-like object
            let bytes_like = ArgBytesLike::try_from_object(vm, buf)?;
            let data = bytes_like.borrow_buf();
            let len = data.len();

            let mut buffer = self.buffer.lock();
            buffer.extend_from_slice(&data);

            Ok(len)
        }

        #[pymethod]
        fn write_eof(&self, _vm: &VirtualMachine) {
            self.eof.store(true, Ordering::Relaxed);
        }

        #[pygetset]
        fn pending(&self) -> usize {
            self.buffer.lock().len()
        }

        #[pygetset]
        fn eof(&self) -> bool {
            // EOF is true only when buffer is empty AND write_eof has been called
            self.buffer.lock().is_empty() && self.eof.load(Ordering::Relaxed)
        }
    }

    #[pyattr]
    #[pyclass(module = "_ssl", name = "SSLSession")]
    #[derive(Debug, PyPayload)]
    struct PySSLSession {
        creation_time: SystemTime,
    }

    impl Representable for PySSLSession {
        #[inline]
        fn repr_str(_zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            Ok("<SSLSession>".to_owned())
        }
    }

    #[pyclass(flags(BASETYPE))]
    impl PySSLSession {
        #[pygetset]
        fn time(&self) -> u64 {
            self.creation_time
                .duration_since(UNIX_EPOCH)
                .expect("BUG: What year this is?!")
                .as_secs()
        }

        #[pygetset]
        fn timeout(&self) -> u64 {
            60 * 60 * 24
        }

        #[pygetset]
        fn ticket_lifetime_hint(&self) -> u64 {
            60 * 60 * 24
        }

        #[pygetset]
        fn id(&self, vm: &VirtualMachine) -> PyBytesRef {
            vm.ctx.new_bytes(vec![0, 1, 2, 3])
        }

        #[pygetset]
        fn has_ticket(&self) -> bool {
            false
        }
    }

    // Windows-specific certificate store enumeration functions
    #[cfg(windows)]
    #[pyfunction]
    fn enum_certificates(
        store_name: PyUtf8StrRef,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<PyObjectRef>> {
        let store_name_str = store_name.as_str();
        let certs =
            vm.allow_threads(|| rustpython_host_env::cert_store::enum_certificates(store_name_str));
        if !certs.had_open_store {
            return Err(vm.new_os_error(format!(
                "failed to open certificate store {store_name_str:?}"
            )));
        }

        let certs = certs.entries.into_iter().map(|c| {
            let cert = vm.ctx.new_bytes(c.der);
            let enc_type = match c.encoding {
                rustpython_host_env::cert_store::EncodingType::X509Asn => vm.new_pyobj("x509_asn"),
                rustpython_host_env::cert_store::EncodingType::Pkcs7Asn => {
                    vm.new_pyobj("pkcs_7_asn")
                }
                rustpython_host_env::cert_store::EncodingType::Other(other) => vm.new_pyobj(other),
            };
            let usage: PyObjectRef = match c.valid_uses {
                Ok(rustpython_host_env::cert_store::CertificateUses::All) => {
                    vm.ctx.new_bool(true).into()
                }
                Ok(rustpython_host_env::cert_store::CertificateUses::Oids(oids)) => {
                    match crate::builtins::PyFrozenSet::from_iter(
                        vm,
                        oids.into_iter().map(|oid| vm.ctx.new_str(oid).into()),
                    ) {
                        Ok(set) => set.into_ref(&vm.ctx).into(),
                        Err(_) => vm.ctx.new_bool(true).into(),
                    }
                }
                Err(_) => vm.ctx.new_bool(true).into(),
            };
            Ok(vm.new_tuple((cert, enc_type, usage)).into())
        });
        certs.collect::<PyResult<Vec<_>>>()
    }

    #[cfg(windows)]
    #[pyfunction]
    fn enum_crls(store_name: PyUtf8StrRef, vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
        let store_name_str = store_name.as_str();
        let crls = vm
            .allow_threads(|| rustpython_host_env::cert_store::enum_crls(store_name_str))
            .map_err(|_| {
                vm.new_os_error(format!(
                    "failed to open certificate store {store_name_str:?}"
                ))
            })?;

        Ok(crls
            .into_iter()
            .map(|crl| {
                let enc_type = match crl.encoding {
                    rustpython_host_env::cert_store::EncodingType::X509Asn => {
                        vm.new_pyobj("x509_asn")
                    }
                    rustpython_host_env::cert_store::EncodingType::Pkcs7Asn => {
                        vm.new_pyobj("pkcs_7_asn")
                    }
                    rustpython_host_env::cert_store::EncodingType::Other(other) => {
                        vm.new_pyobj(other)
                    }
                };
                vm.new_tuple((vm.ctx.new_bytes(crl.der), enc_type)).into()
            })
            .collect())
    }

    #[derive(FromArgs)]
    struct Txt2ObjArgs {
        txt: PyUtf8StrRef,

        #[pyarg(named, optional)]
        name: OptionalArg<bool>,
    }

    #[pyfunction]
    fn txt2obj(args: Txt2ObjArgs, vm: &VirtualMachine) -> PyResult {
        let txt = args.txt.as_str();
        let name = args.name.unwrap_or(false);

        // Lookup by oid first.
        let mut entry = txt
            .split('.')
            .map(|s| s.parse())
            .collect::<Result<Vec<u64>, _>>()
            .ok()
            .and_then(|o| Oid::from(&o).ok())
            .and_then(|o| OID_MAPPINGS.oid_to_entry.get(&o).map(|e| (o, e)));

        if name && entry.is_none() {
            entry = OID_MAPPINGS
                .name_to_oid
                .get(txt)
                .and_then(|o| OID_MAPPINGS.oid_to_entry.get(o).map(|e| (o.clone(), e)))
        }

        let (oid, entry) =
            entry.ok_or_else(|| vm.new_value_error(format!("unknown object '{txt}'")))?;
        let oid_sn = (oid, entry.sn());

        // Return tuple: (nid, shortname, longname, oid)
        Ok(vm
            .new_tuple((
                OID_MAPPINGS
                    .oid_sn_to_nid
                    .get(&oid_sn)
                    .ok_or_else(|| {
                        vm.new_value_error(format!("object '{txt}' does not have a known NID"))
                    })
                    .map(|n| vm.ctx.new_int(*n))?,
                vm.ctx.new_str(entry.sn()),
                vm.ctx.new_str(entry.description()),
                vm.ctx.new_str(oid_sn.0.to_string()),
            ))
            .into())
    }

    #[pyfunction]
    fn nid2obj(nid: i32, vm: &VirtualMachine) -> PyResult {
        let nid = nid
            .try_into()
            .map_err(|_| vm.new_value_error(format!("unknown NID {nid}")))?;
        let oid = OID_MAPPINGS
            .nid_to_oid
            .get(&nid)
            .ok_or_else(|| vm.new_value_error(format!("unknown NID {nid}")))?;
        let entry = OID_MAPPINGS.oid_to_entry.get(oid).expect("BUG: Impossible");

        // Return tuple: (nid, shortname, longname, oid)
        Ok(vm
            .new_tuple((
                vm.ctx.new_int(nid),
                vm.ctx.new_str(entry.sn()),
                vm.ctx.new_str(entry.description()),
                vm.ctx.new_str(oid.to_string()),
            ))
            .into())
    }

    #[pyfunction]
    fn get_default_verify_paths(vm: &VirtualMachine) -> PyTupleRef {
        const DEV_NULL: &str = cfg_select! {
            windows => "nul",
            _ => "/dev/null",
        };

        // Lib/ssl.py expects: (openssl_cafile_env, openssl_cafile, openssl_capath_env, openssl_capath)
        vm.ctx.new_tuple(vec![
            vm.ctx.new_str(CERT_FILE_ENV).into(),
            vm.ctx.new_str(DEV_NULL).into(),
            vm.ctx.new_str(CERT_DIR_ENV).into(),
            vm.ctx.new_str(DEV_NULL).into(),
        ])
    }

    // See `man openssl-env`.
    const CERT_FILE_ENV: &str = "SSL_CERT_FILE";
    const CERT_DIR_ENV: &str = "SSL_CERT_DIR";

    #[pyfunction]
    fn RAND_status() -> bool {
        // Pretend that used RNG always has enough entropy
        // RAND_bytes() will just block if system does not have enough entropy.
        true
    }

    #[pyfunction]
    fn RAND_add(_string: PyObjectRef, _entropy: f64) {
        // There is no way to easily support this.
        // RAND_bytes() will just block if system does not have enough entropy.
    }

    #[pyfunction]
    fn RAND_bytes(len: isize, vm: &VirtualMachine) -> PyResult<PyBytesRef> {
        let len = len
            .try_into()
            .map_err(|_| vm.new_value_error(format!("length is out of range: {len}")))?;

        let rng = CryptoExt::get_provider().secure_random;
        let mut buf = vec![0u8; len];
        vm.allow_threads(|| rng.fill(&mut buf))
            .map_err(|_| vm.new_os_error("Failed to generate random bytes"))?;
        Ok(PyBytesRef::from(vm.ctx.new_bytes(buf)))
    }

    // Used in test_ssl.py.
    #[pyfunction]
    fn _test_decode_cert(path: FsPath, vm: &VirtualMachine) -> PyResult {
        let ders = load_der_bytes_from_pem_or_der_file(
            path.to_path_buf(vm)?,
            &[DerKind::Cert],
            &mut Password::None,
            vm,
        )
        .map_err(|e| e.into_py_err(vm))?;
        let der = ensure_single_der_bytes(&path.to_string_lossy(), ders)
            .map_err(|e| e.into_py_err(vm))?;
        CertInfo::parse_to_py(&der.bytes, vm)
    }

    #[pyfunction]
    fn DER_cert_to_PEM_cert(der_cert: ArgBytesLike, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        let bytes = der_cert.borrow_buf();
        let pem = der_to_pem_cert(&bytes)
            .ok_or_else(|| vm.new_memory_error("certificate is too big for PEM encoding"))?;
        Ok(vm.ctx.new_str(pem))
    }

    #[pyfunction]
    fn PEM_cert_to_DER_cert(pem_cert: PyUtf8StrRef, vm: &VirtualMachine) -> PyResult<PyBytesRef> {
        let ders = load_der_bytes_from_pem_or_der_bytes(
            "<memory>",
            pem_cert.as_bytes().to_vec(),
            &[DerKind::Cert],
            &mut Password::None,
            vm,
        )
        .map_err(|e| e.into_py_err(vm))?;
        let der = ensure_single_der_bytes("<memory>", ders).map_err(|e| e.into_py_err(vm))?;
        Ok(vm.ctx.new_bytes(der.bytes))
    }

    #[pyattr]
    #[pyclass(module = "_ssl", name = "Certificate")]
    #[derive(Debug, PyPayload)]
    struct PySSLCertificate {
        bytes: Vec<u8>,
    }

    // Implement Comparable trait for PySSLCertificate
    impl Comparable for PySSLCertificate {
        fn cmp(
            zelf: &Py<Self>,
            other: &PyObject,
            op: PyComparisonOp,
            _vm: &VirtualMachine,
        ) -> PyResult<PyComparisonValue> {
            op.eq_only(|| {
                if let Some(other_cert) = other.downcast_ref::<Self>() {
                    Ok((zelf.bytes == other_cert.bytes).into())
                } else {
                    Ok(PyComparisonValue::NotImplemented)
                }
            })
        }
    }

    // Implement Hashable trait for PySSLCertificate
    impl Hashable for PySSLCertificate {
        fn hash(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<PyHash> {
            let mut hasher = DefaultHasher::new();
            zelf.bytes.hash(&mut hasher);
            Ok(hasher.finish() as PyHash)
        }
    }

    // Implement Representable trait for PySSLCertificate
    impl Representable for PySSLCertificate {
        fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            Ok(parse_x509_certificate(&zelf.bytes).map_or_else(
                |_| "<Certificate(invalid)>".to_string(),
                |(_, c)| format!("<Certificate(subject={})>", c.subject()),
            ))
        }
    }

    #[pyclass(with(Comparable, Hashable, Representable))]
    impl PySSLCertificate {
        #[pymethod]
        fn public_bytes(&self, format: OptionalArg<i32>, vm: &VirtualMachine) -> PyResult {
            let format = format.unwrap_or(ENCODING_PEM);

            match format {
                ENCODING_DER => Ok(vm.ctx.new_bytes(self.bytes.clone()).into()),

                ENCODING_PEM => {
                    let pem = der_to_pem_cert(&self.bytes).ok_or_else(|| {
                        vm.new_memory_error("certificate is too big for PEM encoding")
                    })?;
                    Ok(vm.ctx.new_str(pem).into())
                }

                _ => Err(vm.new_value_error("Unsupported format")),
            }
        }

        #[pymethod]
        fn get_info(&self, vm: &VirtualMachine) -> PyResult {
            CertInfo::parse_to_py(&self.bytes, vm)
        }
    }
}

//
// Connection state.
//

enum State {
    ServerWaitingForClientHello(Acceptor),

    ServerSendingAlert {
        error: PyBaseExceptionRef,
        alert_buf: [u8; TLS_RECORD_HEADER_LEN + TLS_ALERT_RECORD_LEN],
        alert_buf_pos: usize,
    },

    HasConnection {
        state: ConnectionState,
        conn: Connection,
    },
}

#[derive(Debug)]
enum ConnectionState {
    Handshaking,
    Connected(CloseNotifyState),
    ShuttingDown,
    ShutDown,
}

#[derive(Debug)]
enum CloseNotifyState {
    None,
    Received,
    Sent,
}

impl core::fmt::Debug for State {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ServerWaitingForClientHello(_) => f
                .debug_tuple("ServerWaitingForClientHello")
                .field(&"Acceptor")
                .finish(),

            Self::ServerSendingAlert { error, .. } => f
                .debug_struct("ServerSendingAlert")
                .field("error", error)
                .finish(),

            Self::HasConnection { state, conn } => f
                .debug_struct("Handshaking")
                .field("state", state)
                .field("conn", conn)
                .finish(),
        }
    }
}

impl State {
    fn new_handshaking_server() -> Self {
        Self::ServerWaitingForClientHello(Acceptor::default())
    }

    fn new_handshaking_client(conn: Connection) -> Self {
        Self::HasConnection {
            state: ConnectionState::Handshaking,
            conn,
        }
    }

    fn new_alert_from_rustls_error(
        error: rustls::Error,
        mut alert: AcceptedAlert,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        let mut alert_buf = [0u8; TLS_RECORD_HEADER_LEN + TLS_ALERT_RECORD_LEN];
        let mut alert_buf_mut: &mut [u8] = &mut alert_buf;

        if alert.write_all(&mut alert_buf_mut).is_err() || !alert_buf_mut.is_empty() {
            Err(SslError::Ssl("TLS alert is too long or too short".to_string()).into_py_err(vm))
        } else {
            Ok(Self::ServerSendingAlert {
                error: SslError::from_rustls(error).into_py_err(vm),
                alert_buf,
                alert_buf_pos: 0,
            })
        }
    }

    fn new_alert_from_sni_callback_error(error: PyBaseExceptionRef, alert_code: u8) -> Self {
        Self::ServerSendingAlert {
            error,

            #[rustfmt::skip]
            alert_buf: [
                0x15,       // type == alert
                0x03, 0x03, // version == TLS 1.2 (TODO: Is it fine that we hardcode TLS 1.2 here?)
                0x00, 0x02, // length == 2 bytes
                0x02,       // alert level == fatal
                alert_code, // code returned by SNI callback
            ],

            alert_buf_pos: 0,
        }
    }

    fn get_connection(&self) -> Option<&Connection> {
        match self {
            Self::ServerWaitingForClientHello(_)
            | Self::ServerSendingAlert { .. }
            | Self::HasConnection {
                state: ConnectionState::Handshaking,
                ..
            } => None,

            Self::HasConnection {
                state:
                    ConnectionState::Connected(_)
                    | ConnectionState::ShuttingDown
                    | ConnectionState::ShutDown,
                conn,
            } => Some(conn),
        }
    }

    fn get_connection_mut(&mut self) -> Option<&mut Connection> {
        match self {
            Self::ServerWaitingForClientHello(_)
            | Self::ServerSendingAlert { .. }
            | Self::HasConnection {
                state: ConnectionState::Handshaking,
                ..
            } => None,

            Self::HasConnection {
                state:
                    ConnectionState::Connected(_)
                    | ConnectionState::ShuttingDown
                    | ConnectionState::ShutDown,
                conn,
            } => Some(conn),
        }
    }
}

//
// IO wrapper.
//

#[derive(Debug)]
struct Io {
    // TODO: Support timeouts.
    socket_or_bio: SocketOrBio,
    hdr: [u8; TLS_RECORD_HEADER_LEN],
    hdr_len: usize,
}

const TLS_RECORD_HEADER_LEN: usize = 5;
const TLS_ALERT_RECORD_LEN: usize = 2;

#[derive(Debug)]
enum SocketOrBio {
    Socket {
        socket: PyObjectRef,

        // TODO: Investigate why normal `sock.send()`/`sock.recv()` lead to a hang.
        sock_send_method: PyObjectRef,
        sock_recv_method: PyObjectRef,
    },

    Bio {
        incoming: PyObjectRef,
        outgoing: PyObjectRef,
    },
}

impl Io {
    fn from_socket(socket: PyObjectRef, vm: &VirtualMachine) -> PyResult<Self> {
        // TODO: Call send() and recv() directly. Currently this deadlocks for some reason.
        let socket_mod = vm.import("socket", 0)?;
        let socket_class = socket_mod.get_attr("socket", vm)?;
        Ok(Self {
            socket_or_bio: SocketOrBio::Socket {
                socket,
                sock_send_method: socket_class.get_attr("send", vm)?,
                sock_recv_method: socket_class.get_attr("recv", vm)?,
            },
            hdr: [0; TLS_RECORD_HEADER_LEN],
            hdr_len: 0,
        })
    }

    fn from_bio(incoming: PyObjectRef, outgoing: PyObjectRef) -> Self {
        Self {
            socket_or_bio: SocketOrBio::Bio { incoming, outgoing },
            hdr: [0; TLS_RECORD_HEADER_LEN],
            hdr_len: 0,
        }
    }

    fn with_io<F, T>(&mut self, vm: &VirtualMachine, f: F) -> PyResult<T>
    where
        F: FnOnce(&mut WithIo<'_>) -> std::io::Result<T>,
    {
        let mut io = WithIo {
            io: self,
            vm,
            error: None,
        };
        match f(&mut io) {
            Ok(value) => Ok(value),

            Err(err) => match err.kind() {
                std::io::ErrorKind::Other => {
                    Err(io.error.take().expect("BUG: Io.error is not set"))
                }

                std::io::ErrorKind::InvalidData => {
                    // ConnectionCommon::complete_io() wraps TLS processing errors in InvalidData.
                    let err = err
                        .downcast::<rustls::Error>()
                        .expect("BUG: Not a rustls Error");
                    Err(SslError::from_rustls(err).into_py_err(vm))
                }

                _ => Err(SslError::Io(err).into_py_err(vm)),
            },
        }
    }

    fn to_socket(&self, vm: &VirtualMachine) -> PyObjectRef {
        match &self.socket_or_bio {
            SocketOrBio::Socket { socket, .. } => socket.clone(),
            SocketOrBio::Bio { .. } => vm.ctx.none(),
        }
    }
}

struct WithIo<'a> {
    io: &'a mut Io,
    vm: &'a VirtualMachine,
    error: Option<PyBaseExceptionRef>,
}

impl std::io::Read for WithIo<'_> {
    // Read no more than a single TLS entry.
    // TODO: Wait for better unbuffered API in rustls.
    //     See https://github.com/rustls/rustls/pull/2905
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }

        if self.io.hdr_len < TLS_RECORD_HEADER_LEN {
            // We do not have a full TLS record header, start receiving one.
            let len = buf.len().min(TLS_RECORD_HEADER_LEN - self.io.hdr_len);
            let buf = &mut buf[..len];
            let read = self.read_inner(buf)?;
            self.io.hdr[self.io.hdr_len..self.io.hdr_len + len].copy_from_slice(buf);
            self.io.hdr_len += read;

            if self.io.hdr_len == TLS_RECORD_HEADER_LEN {
                // Parse the body length.
                let record_body_len = u16::from_be_bytes([self.io.hdr[3], self.io.hdr[4]]);

                // Zero-length TLS record.
                if record_body_len == 0 {
                    self.io.hdr_len = 0;
                }
            }

            Ok(read)
        } else {
            // Parse the body length.
            let mut record_body_len = u16::from_be_bytes([self.io.hdr[3], self.io.hdr[4]]);
            // Validity of length value will be checked by rustls.
            let buf_len = buf.len();
            let buf = &mut buf[..buf_len.min(record_body_len.into())];

            let read = self.read_inner(buf)?;

            record_body_len -= read as u16;
            if record_body_len == 0 {
                // Start reading next record.
                self.io.hdr_len = 0;
            } else {
                // Update remaining length in the header.
                self.io.hdr.as_mut_slice()[3..5].copy_from_slice(&record_body_len.to_be_bytes());
            }

            Ok(read)
        }
    }
}

impl std::io::Write for WithIo<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }

        let res = match &self.io.socket_or_bio {
            SocketOrBio::Socket {
                socket,

                sock_send_method,
                ..
            } => sock_send_method.call(
                (socket.clone(), self.vm.ctx.new_bytes(buf.to_vec())),
                self.vm,
            ),

            SocketOrBio::Bio { outgoing, .. } => outgoing
                .get_attr("write", self.vm)
                .and_then(|w| w.call((self.vm.ctx.new_bytes(buf.to_vec()),), self.vm)),
        }
        .and_then(|b| usize::try_from_object(self.vm, b));

        match res {
            Ok(len) => Ok(len),

            Err(err) => {
                assert!(self.error.is_none(), "BUG: Duplicate error");

                if err.fast_isinstance(self.vm.ctx.exceptions.blocking_io_error) {
                    self.error = Some(SslError::WantWrite.into_py_err(self.vm));
                    Err(std::io::Error::other("SSLWantWriteError"))
                } else {
                    self.error = Some(err);
                    Err(std::io::Error::other("Python IO error when writing"))
                }
            }
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        // Neither socket nor buffer IO need this.
        Ok(())
    }
}

impl WithIo<'_> {
    fn read_inner(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self.read_inner_py(buf.len()) {
            Ok(Some(bytes)) => {
                if bytes.is_empty() {
                    // Zero read means EOF.
                    self.error = Some(SslError::Eof.into_py_err(self.vm));
                    Err(std::io::Error::other("SSLEOFError"))
                } else {
                    let bytes = bytes.borrow_buf();
                    buf[..bytes.len()].copy_from_slice(&bytes);
                    Ok(bytes.len())
                }
            }

            Ok(None) => {
                assert!(self.error.is_none(), "BUG: Duplicate error");
                self.error = Some(SslError::WantRead.into_py_err(self.vm));
                Err(std::io::Error::other("SSLWantReadError"))
            }

            Err(err) => {
                assert!(self.error.is_none(), "BUG: Duplicate error");
                self.error = Some(err);
                Err(std::io::Error::other("Python IO error when reading"))
            }
        }
    }

    fn read_inner_py(&mut self, len: usize) -> PyResult<Option<ArgBytesLike>> {
        let res = match &self.io.socket_or_bio {
            SocketOrBio::Socket {
                socket,

                sock_recv_method,
                ..
            } => sock_recv_method.call((socket.clone(), self.vm.ctx.new_int(len)), self.vm),

            SocketOrBio::Bio { incoming, .. } => incoming
                .get_attr("read", self.vm)
                .and_then(|r| r.call((self.vm.ctx.new_int(len),), self.vm)),
        }
        .and_then(|b| ArgBytesLike::try_from_object(self.vm, b));

        if let SocketOrBio::Bio { incoming, .. } = &self.io.socket_or_bio {
            let bytes = res?;
            if bytes.is_empty() {
                let eof = incoming.get_attr("eof", self.vm)?;
                let eof = bool::try_from_object(self.vm, eof)?;
                if eof { Ok(Some(bytes)) } else { Ok(None) }
            } else {
                Ok(Some(bytes))
            }
        } else {
            match res {
                Ok(bytes) => Ok(Some(bytes)),

                Err(err) if err.fast_isinstance(self.vm.ctx.exceptions.blocking_io_error) => {
                    Ok(None)
                }

                Err(err) => Err(err),
            }
        }
    }
}

//
// Cipher info.
//

#[derive(Serialize)]
struct CipherDescriptionDict {
    id: u16,
    name: &'static str,
    protocol: &'static str,
    description: &'static str,
    strength_bits: u16,
    alg_bits: u16,
}

impl CipherDescriptionDict {
    fn new(cipher: &SupportedCipherSuite) -> Self {
        let id = cipher.suite().into();
        let bits = CIPHER_MAPPINGS.id_to_bits[&id];
        Self {
            id,
            name: CIPHER_MAPPINGS.id_to_openssl[&id],

            protocol: match cipher.version().version {
                ProtocolVersion::TLSv1_2 => "TLSv1.2",
                ProtocolVersion::TLSv1_3 => "TLSv1.3",

                // This is tested by that_all_rustls_tls_versions_are_known().
                // This may happen after rustls update, just add more ciphers above is this case.
                version => unreachable!("BUG: Unknown TLS version {version:?}"),
            },

            description: CIPHER_MAPPINGS.id_to_openssl[&id],
            strength_bits: bits,
            alg_bits: bits,
        }
    }
}

fn cipher_to_tuple(cipher: &SupportedCipherSuite, vm: &VirtualMachine) -> PyTupleRef {
    let id = cipher.suite().into();

    vm.ctx.new_tuple(vec![
        vm.ctx
            .new_str(CIPHER_MAPPINGS.id_to_openssl[&id])
            .into_object(),
        vm.ctx.new_str(cipher_to_version(cipher)).into_object(),
        vm.ctx
            .new_int(CIPHER_MAPPINGS.id_to_bits[&id])
            .into_object(),
    ])
}

fn cipher_to_version(cipher: &SupportedCipherSuite) -> &'static str {
    match cipher.version().version {
        ProtocolVersion::TLSv1_2 => "TLSv1.2",
        ProtocolVersion::TLSv1_3 => "TLSv1.3",
        _ => "unknown",
    }
}

//
// PEM, DER, certificate and private key utilities.
//

fn ensure_single_der_bytes(path_str: &str, mut ders: Vec<DerBytes>) -> SslResult<DerBytes> {
    let mut ders = ders.drain(..);
    let der = ders.next().expect("BUG: Impossible");
    if ders.next().is_some() {
        return Err(SslError::Ssl(format!(
            "more than one certificate in {path_str}"
        )));
    }
    Ok(der)
}

fn load_der_bytes_from_pem_or_der_file(
    path: impl AsRef<Path>,
    kinds: &[DerKind],
    password: &mut Password,
    vm: &VirtualMachine,
) -> SslResult<Vec<DerBytes>> {
    load_der_bytes_from_pem_or_der_file_inner(path.as_ref(), kinds, password, vm)
}

fn load_der_bytes_from_pem_or_der_file_inner(
    path: &Path,
    kinds: &[DerKind],
    password: &mut Password,
    vm: &VirtualMachine,
) -> SslResult<Vec<DerBytes>> {
    let bytes = vm
        .allow_threads(|| rustpython_host_env::fs::read(path))
        .map_err(SslError::Io)?;
    load_der_bytes_from_pem_or_der_bytes(&format!("{path:?}"), bytes, kinds, password, vm)
}

// This function does not verify that returned DER data is correct.
// rustls or x509_parser will check for correctness later.
fn load_der_bytes_from_pem_or_der_bytes(
    path_str: &str,
    bytes: Vec<u8>,
    kinds: &[DerKind],
    password: &mut Password,
    vm: &VirtualMachine,
) -> SslResult<Vec<DerBytes>> {
    assert!(!kinds.is_empty(), "BUG Empty PEM/DER kinds");

    let (mut ders, first_pem_entry_not_read) =
        load_der_bytes_from_pem(path_str, &bytes, kinds, password, vm)?;

    if first_pem_entry_not_read {
        // PEM reading failed right away so this must be DER (possibly more than one
        // DER-encoded object in the same file).
        ders = load_der_bytes_from_der(path_str, &bytes, kinds, password, vm)?;
    }

    if ders.is_empty() {
        Err(SslError::PemLib("no PEM certificates found".to_string()))
    } else {
        Ok(ders)
    }
}

fn load_der_bytes_from_pem(
    path_str: &str,
    bytes: &[u8],
    kinds: &[DerKind],
    password: &mut Password,
    vm: &VirtualMachine,
) -> SslResult<(Vec<DerBytes>, bool)> {
    let mut ders = Vec::new();
    let mut first_pem_entry_not_read = true;
    for pem in Pem::iter_from_buffer(bytes) {
        if first_pem_entry_not_read {
            if pem.is_err() {
                break;
            }
            first_pem_entry_not_read = false;
        }
        let pem = pem.map_err(|e| SslError::PemLib(e.to_string()))?;

        let (kind, bytes) = match pem.label.as_str() {
            "CERTIFICATE" | "TRUSTED CERTIFICATE" if kinds.contains(&DerKind::Cert) => {
                (DerKind::Cert, pem.contents)
            }

            "CERTIFICATE REVOCATION LIST" | "X509 CRL" if kinds.contains(&DerKind::Crl) => {
                (DerKind::Crl, pem.contents)
            }

            "PRIVATE KEY" | "EC PRIVATE KEY" | "RSA PRIVATE KEY"
                if kinds.contains(&DerKind::Key) =>
            {
                (DerKind::Key, pem.contents)
            }

            "ENCRYPTED PRIVATE KEY" if kinds.contains(&DerKind::Key) => (
                DerKind::Key,
                decrypt_private_key(path_str, &pem.contents, password, vm)?.1,
            ),

            _ => continue,
        };
        ders.push(DerBytes { kind, bytes });
    }

    Ok((ders, first_pem_entry_not_read))
}

fn load_der_bytes_from_der(
    path_str: &str,
    mut bytes: &[u8],
    kinds: &[DerKind],
    password: &mut Password,
    vm: &VirtualMachine,
) -> SslResult<Vec<DerBytes>> {
    let mut ders = Vec::new();
    while !bytes.is_empty() {
        let mut last_error = None;
        for kind in kinds {
            match kind {
                DerKind::Key => match decrypt_private_key(path_str, bytes, password, vm) {
                    Ok((rem, parsed_bytes)) => {
                        bytes = rem;
                        ders.push(DerBytes {
                            kind: DerKind::Key,
                            bytes: parsed_bytes,
                        });
                        last_error = None;
                        break;
                    }

                    Err(err) => last_error = Some(err),
                },

                DerKind::Crl => match parse_x509_crl(bytes) {
                    Ok((rem, crl)) => {
                        bytes = rem;
                        ders.push(DerBytes {
                            kind: DerKind::Crl,
                            bytes: crl.as_raw().to_vec(),
                        });
                        last_error = None;
                        break;
                    }

                    Err(err) => {
                        last_error = Some(SslError::FailedToReadDer(format!(
                            "certificate revocation list from {path_str}: {err}"
                        )))
                    }
                },

                DerKind::Cert => match parse_x509_certificate(bytes) {
                    Ok((rem, cert)) => {
                        bytes = rem;
                        ders.push(DerBytes {
                            kind: DerKind::Cert,
                            bytes: cert.as_raw().to_vec(),
                        });
                        last_error = None;
                        break;
                    }

                    Err(err) => {
                        last_error = Some(SslError::FailedToReadDer(format!(
                            "certificate from {path_str}: {err}"
                        )))
                    }
                },
            }
        }

        if let Some(err) = last_error {
            return Err(err);
        }
    }
    Ok(ders)
}

struct DerBytes {
    kind: DerKind,
    bytes: Vec<u8>,
}

#[derive(Eq, PartialEq, Clone, Copy)]
enum DerKind {
    Cert,
    Crl,
    Key,
}

fn decrypt_private_key<'a>(
    path_str: &str,
    bytes: &'a [u8],
    password: &mut Password,
    vm: &VirtualMachine,
) -> SslResult<(&'a [u8], Vec<u8>)> {
    // Try to parse as encrypted private key and keep any trailing data.
    let mut aligned_bytes = bytes;
    let rem_plus_encrypted = loop {
        match EncryptedPrivateKeyInfoRef::from_der(aligned_bytes) {
            Ok(encrypted) => break Some((&bytes[aligned_bytes.len()..], encrypted)),

            Err(err) => {
                if let pkcs8::der::ErrorKind::TrailingData { decoded, .. } = err.kind() {
                    aligned_bytes = &aligned_bytes[..decoded.try_into().unwrap()]
                } else {
                    break None;
                }
            }
        }
    };

    if let Some((rem, encrypted)) = rem_plus_encrypted {
        // Try to decrypt
        let password = password.password(vm).map_err(SslError::Py)?;
        let decrypted = encrypted.decrypt(password).map_err(|e| {
            SslError::Ssl(format!(
                "failed to decrypt private key from {path_str}: {e}"
            ))
        })?;
        Ok((rem, decrypted.as_bytes().to_vec()))
    } else {
        // Parse as plain text private key and keep any trailing data.
        let mut aligned_bytes = bytes;
        let rem = loop {
            match PrivateKeyInfoRef::from_der(aligned_bytes) {
                Ok(_) => break &bytes[aligned_bytes.len()..],

                Err(err) => {
                    if let pkcs8::der::ErrorKind::TrailingData { decoded, .. } = err.kind() {
                        aligned_bytes = &aligned_bytes[..decoded.try_into().unwrap()]
                    } else {
                        return Err(SslError::Ssl(format!(
                            "invalid private key in {path_str}: {err}"
                        )));
                    }
                }
            }
        };
        Ok((rem, bytes[..bytes.len() - rem.len()].to_vec()))
    }
}

enum Password {
    None,
    Callable(PyObjectRef),
    Bytes(Vec<u8>),
}

impl Password {
    const MAX_PASSWORD_LEN: usize = 1024;

    fn new(password: OptionalArg<PyObjectRef>, vm: &VirtualMachine) -> PyResult<Self> {
        let password = match password {
            OptionalArg::Missing => return Ok(Self::None),
            OptionalArg::Present(password) => password,
        };

        if vm.is_none(&password) {
            Ok(Self::None)
        } else if password.is_callable() {
            Ok(Self::Callable(password))
        } else if let Ok(password) = ArgBytesLike::try_from_object(vm, password.clone()) {
            Ok(Self::Bytes(Self::validate(
                password.borrow_buf().to_vec(),
                vm,
            )?))
        } else if let Ok(password) = PyUtf8StrRef::try_from_object(vm, password) {
            Ok(Self::Bytes(Self::validate(
                password.as_str().as_bytes().to_vec(),
                vm,
            )?))
        } else {
            Err(vm.new_type_error("password should be a string or callable"))
        }
    }

    fn password(&mut self, vm: &VirtualMachine) -> PyResult<&[u8]> {
        match self {
            // TODO: Prompt user for password.
            Self::None => Err(vm.new_value_error("no password provided")),
            Self::Bytes(bytes) => Ok(bytes.as_slice()),

            Self::Callable(callable) => {
                let password = callable.call((), vm)?;
                if let Ok(password) = ArgBytesLike::try_from_object(vm, password.clone()) {
                    *self = Self::Bytes(Self::validate(password.borrow_buf().to_vec(), vm)?);
                    // TODO: Rewrite without recursion?
                    self.password(vm)
                } else if let Ok(password) = PyUtf8StrRef::try_from_object(vm, password) {
                    *self = Self::Bytes(Self::validate(password.as_str().as_bytes().to_vec(), vm)?);
                    // TODO: Rewrite without recursion?
                    self.password(vm)
                } else {
                    Err(vm.new_type_error("password callback must return a string"))
                }
            }
        }
    }

    fn validate(bytes: Vec<u8>, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        if bytes.len() > Self::MAX_PASSWORD_LEN {
            Err(vm.new_value_error(format!(
                "password cannot be longer than {} bytes",
                Self::MAX_PASSWORD_LEN
            )))
        } else {
            Ok(bytes)
        }
    }
}

fn der_to_pem_cert(der: &[u8]) -> Option<String> {
    // TODO: Encode line by line to consume less memory.
    const MAX_LINE_LEN: usize = 64;

    let len = base64::encoded_len(der.len(), true)?;
    let mut enc_buf = String::with_capacity(len);
    BASE64_STANDARD.encode_string(der, &mut enc_buf);

    let mut buf = String::with_capacity(len + (len / MAX_LINE_LEN) + 100);
    buf.push_str("-----BEGIN CERTIFICATE-----\n");
    for line in enc_buf
        .as_bytes()
        .chunks(MAX_LINE_LEN)
        .map(|b| str::from_utf8(b).expect("BUG: Impossible"))
    {
        buf.push_str(line);
        buf.push('\n');
    }
    buf.push_str("-----END CERTIFICATE-----\n");
    Some(buf)
}

#[allow(non_snake_case)]
#[derive(Serialize)]
struct CertInfo {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    OCSP: Vec<String>,

    #[serde(skip_serializing_if = "Vec::is_empty")]
    caIssuers: Vec<String>,

    #[serde(skip_serializing_if = "Vec::is_empty")]
    crlDistributionPoints: Vec<String>,

    #[serde(skip_serializing_if = "Vec::is_empty")]
    issuer: Vec<((String, String),)>,

    notAfter: String,
    notBefore: String,
    serialNumber: String,

    #[serde(skip_serializing_if = "Vec::is_empty")]
    subject: Vec<((String, String),)>,

    #[serde(skip_serializing_if = "Vec::is_empty")]
    subjectAltName: Vec<CertInfoPairOrNested>,

    version: u32,
}

#[derive(Serialize)]
#[serde(untagged)]
enum CertInfoPairOrNested {
    Pair(&'static str, String),
    Nested(&'static str, Vec<((String, String),)>),
}

impl CertInfo {
    fn parse_to_py(bytes: &[u8], vm: &VirtualMachine) -> PyResult {
        let cert =
            Self::parse(bytes).map_err(|_| vm.new_value_error("failed to parse certificate"))?;
        vm.with_serde_conf(RustPySerDeConf::default().lists_as_tuples(), |serde| {
            cert.serialize(serde)
        })
    }

    fn parse(bytes: &[u8]) -> Result<Self, &'static str> {
        let (_, cert) = parse_x509_certificate(bytes).map_err(|_| "failed to parse certificate")?;

        let tbs_exts = cert
            .tbs_certificate
            .extensions_map()
            .map_err(|_| "duplicate TBSCertificate extension")?;

        // CA issuers and OCSP URLs
        let mut ocsp_urls = Vec::new();
        let mut issuer_urls = Vec::new();
        if let Some(ext) = tbs_exts.get(&OID_PKIX_AUTHORITY_INFO_ACCESS) {
            let ext = if let ParsedExtension::AuthorityInfoAccess(ext) = &ext.parsed_extension() {
                ext
            } else {
                return Err("wrong data in authorityInfoAccess extension");
            };
            for desc in &ext.accessdescs {
                let uri = if let GeneralName::URI(uri) = &desc.access_location {
                    uri
                } else {
                    // We are interested in URIs only
                    continue;
                };
                if desc.access_method == OID_PKIX_ACCESS_DESCRIPTOR_OCSP {
                    ocsp_urls.push(uri.to_string());
                } else if desc.access_method == OID_PKIX_ACCESS_DESCRIPTOR_CA_ISSUERS {
                    issuer_urls.push(uri.to_string());
                }
                // Ignore other access methods.
            }
        }

        // CRL distribution points
        let mut crl_urls = Vec::new();
        if let Some(ext) = tbs_exts.get(&OID_X509_EXT_CRL_DISTRIBUTION_POINTS) {
            let ext = if let ParsedExtension::CRLDistributionPoints(ext) = &ext.parsed_extension() {
                ext
            } else {
                return Err("wrong data in cRLDistributionPoints extension");
            };
            for point in ext
                .points
                .iter()
                .filter_map(|p| p.distribution_point.as_ref())
            {
                let names = if let DistributionPointName::FullName(names) = point {
                    names
                } else {
                    continue;
                };
                for name in names {
                    if let GeneralName::URI(uri) = name {
                        crl_urls.push(uri.to_string());
                    }
                }
            }
        }

        // Serial number
        let mut serial_number = cert.serial.to_str_radix(16).to_uppercase();
        if serial_number.len() % 2 == 1 {
            serial_number.insert(0, '0');
        }

        // Alternative URLs
        let alt_names = if let Some(alt_names) = cert
            .subject_alternative_name()
            .map_err(|_| "Subject Alternative Name extension is invalid")?
        {
            alt_names
                .value
                .general_names
                .iter()
                .map(|alt_name| {
                    match alt_name {
                        GeneralName::DNSName(dns) => {
                            Ok(CertInfoPairOrNested::Pair("DNS", dns.to_string()))
                        }

                        GeneralName::IPAddress(ip) => {
                            let ip_str = match ip.len() {
                                4 => format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]),

                                16 => format!(
                                    "{:X}:{:X}:{:X}:{:X}:{:X}:{:X}:{:X}:{:X}",
                                    (u16::from(ip[0]) << 8) | u16::from(ip[1]),
                                    (u16::from(ip[2]) << 8) | u16::from(ip[3]),
                                    (u16::from(ip[4]) << 8) | u16::from(ip[5]),
                                    (u16::from(ip[6]) << 8) | u16::from(ip[7]),
                                    (u16::from(ip[8]) << 8) | u16::from(ip[9]),
                                    (u16::from(ip[10]) << 8) | u16::from(ip[11]),
                                    (u16::from(ip[12]) << 8) | u16::from(ip[13]),
                                    (u16::from(ip[14]) << 8) | u16::from(ip[15]),
                                ),

                                _ => return Err("invalid length of IPv4/IPv6 address"),
                            };
                            Ok(CertInfoPairOrNested::Pair("IP Address", ip_str))
                        }

                        GeneralName::RFC822Name(email) => {
                            Ok(CertInfoPairOrNested::Pair("email", email.to_string()))
                        }

                        GeneralName::URI(uri) => {
                            Ok(CertInfoPairOrNested::Pair("URI", uri.to_string()))
                        }

                        GeneralName::OtherName(_oid, _data) => Ok(CertInfoPairOrNested::Pair(
                            "othername",
                            //format!("{}={}", oid.to_string(), hex::encode(data)),
                            // Python tests actually expect `<unsupported>`...
                            "<unsupported>".to_string(),
                        )),

                        GeneralName::DirectoryName(name) => Ok(CertInfoPairOrNested::Nested(
                            "DirName",
                            Self::name_to_vec(name)?,
                        )),

                        GeneralName::RegisteredID(oid) => {
                            // Convert OID to string representation
                            let oid_str = oid.to_id_string();
                            Ok(CertInfoPairOrNested::Pair("Registered ID", oid_str))
                        }

                        _ => Err("Unknown type of Subject Alternative Name"),
                    }
                })
                .collect::<Result<_, _>>()?
        } else {
            vec![]
        };

        Ok(Self {
            OCSP: ocsp_urls,
            caIssuers: issuer_urls,
            crlDistributionPoints: crl_urls,
            issuer: Self::name_to_vec(&cert.issuer)?,
            notAfter: Self::datetime_to_string(&cert.validity.not_after)?,
            notBefore: Self::datetime_to_string(&cert.validity.not_before)?,
            serialNumber: serial_number,
            subject: Self::name_to_vec(&cert.subject)?,
            subjectAltName: alt_names,
            version: cert.version.0 + 1,
        })
    }

    fn name_to_vec(name: &X509Name<'_>) -> Result<Vec<((String, String),)>, &'static str> {
        let mut entries = Vec::with_capacity(8);
        for rdn in name.iter() {
            for attr in rdn.iter() {
                let attr_name = OID_MAPPINGS
                    .oid_to_entry
                    .get(attr.attr_type())
                    .ok_or("unknown attribute in X509Name")?
                    .description();
                let attr_value = attr
                    .attr_value()
                    .as_str()
                    .or_else(|_| str::from_utf8(attr.attr_value().data))
                    .map_err(|_| "attribute value of X509Name is not a valid UTF-8")?;

                entries.push(((attr_name.to_string(), attr_value.to_string()),));
            }
        }
        Ok(entries)
    }

    fn datetime_to_string(date_time: &ASN1Time) -> Result<String, &'static str> {
        Ok(DateTime::<Utc>::from_timestamp(date_time.timestamp(), 0)
            .ok_or("ASN1Time is not valid")?
            .format("%b %e %H:%M:%S %Y GMT")
            .to_string())
    }
}

//
// Custom certificate verifiers.
//

const VERIFY_CRL_CHECK_LEAF: i32 = 0x00000004;
const VERIFY_CRL_CHECK_CHAIN: i32 = 0x0000000c;

#[derive(Debug)]
struct CustomServerCertVerifier {
    verify_server_certificates: bool,
    verifiers: Vec<Arc<dyn ServerCertVerifier>>,
    check_hostname: bool,
    root_hint_subjects: Vec<DistinguishedName>,
    crl_check_enabled_and_no_platform_verifier_and_no_crl_loaded: bool,
}

#[derive(Debug)]
enum CrlCheck {
    None,
    Leaf,
    Chain,
}

impl ServerCertVerifier for CustomServerCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if !self.verify_server_certificates {
            // Server cert verification disabled.
            return Ok(ServerCertVerified::assertion());
        }

        if self.crl_check_enabled_and_no_platform_verifier_and_no_crl_loaded {
            // cpython's ssl rejects all certificates if CRL check is requested
            // but no CRL loaded.
            return Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::UnknownRevocationStatus,
            ));
        }

        let server_name = if !self.check_hostname
            && let Some(server_name) = Self::first_server_name(end_entity)
        {
            // Substitute real server name with a name extracted from a server-provided
            // certificate to circumvent rustls's server name check if SSLContext.check_hostname
            // is False.
            server_name
        } else {
            server_name.clone()
        };

        let mut last_ok = None;
        for verifier in &self.verifiers {
            let res = verifier.verify_server_cert(
                end_entity,
                intermediates,
                &server_name,
                ocsp_response,
                now,
            );

            // Certificate is valid if at least one of verifiers report it as valid and other
            // verifiers report "unknown issuer" because they do not have a matching root certificate.
            match res {
                Err(rustls::Error::InvalidCertificate(rustls::CertificateError::UnknownIssuer)) => {
                }

                Ok(verified) => last_ok = Some(verified),

                Err(err) => return Err(err), // any other error from any verifier means that certificate is invalid
            }
        }

        if let Some(verified) = last_ok.take() {
            Ok(verified)
        } else {
            // No verifiers but verification required.
            Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::UnknownIssuer,
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        if !self.verify_server_certificates {
            // Server cert verification disabled.
            return Ok(HandshakeSignatureValid::assertion());
        }

        self.verifiers
            .first()
            .ok_or(rustls::Error::InvalidCertificate(
                rustls::CertificateError::BadSignature,
            ))?
            .verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        if !self.verify_server_certificates {
            // Server cert verification disabled.
            return Ok(HandshakeSignatureValid::assertion());
        }

        self.verifiers
            .first()
            .ok_or(rustls::Error::InvalidCertificate(
                rustls::CertificateError::BadSignature,
            ))?
            .verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        let mut schemes = Vec::new();

        if self.verifiers.is_empty() {
            // Provide some default list when we are either not really verifying anything or reject everything.
            schemes.extend_from_slice(&[
                SignatureScheme::RSA_PKCS1_SHA1,
                SignatureScheme::ECDSA_SHA1_Legacy,
                SignatureScheme::RSA_PKCS1_SHA256,
                SignatureScheme::ECDSA_NISTP256_SHA256,
                SignatureScheme::RSA_PKCS1_SHA384,
                SignatureScheme::ECDSA_NISTP384_SHA384,
                SignatureScheme::RSA_PKCS1_SHA512,
                SignatureScheme::ECDSA_NISTP521_SHA512,
                SignatureScheme::RSA_PSS_SHA256,
                SignatureScheme::RSA_PSS_SHA384,
                SignatureScheme::RSA_PSS_SHA512,
                SignatureScheme::ED25519,
                SignatureScheme::ED448,
                SignatureScheme::ML_DSA_44,
                SignatureScheme::ML_DSA_65,
                SignatureScheme::ML_DSA_87,
            ]);
        } else {
            // Intersection of sets.
            for (i, verifier) in self.verifiers.iter().enumerate() {
                if i == 0 {
                    schemes.extend_from_slice(&verifier.supported_verify_schemes())
                } else {
                    let other_schemes = verifier.supported_verify_schemes();
                    schemes.retain(|s| other_schemes.contains(s));
                }
            }
        }

        schemes
    }

    fn requires_raw_public_keys(&self) -> bool {
        self.verifiers.iter().any(|v| v.requires_raw_public_keys())
    }

    fn root_hint_subjects(&self) -> Option<&[DistinguishedName]> {
        if self.root_hint_subjects.is_empty() {
            None
        } else {
            Some(&self.root_hint_subjects)
        }
    }
}

impl CustomServerCertVerifier {
    fn new(
        verify_server_certificates: bool,
        use_system_certificates: bool,
        cert_store: &CertStore,
        crypto: Arc<CryptoProvider>,
        check_hostname: bool,
        crl_check: CrlCheck,
    ) -> SslResult<Self> {
        if !verify_server_certificates {
            // Server cert verification disabled.
            return Ok(Self {
                verify_server_certificates: false,
                verifiers: vec![],
                check_hostname: false,
                root_hint_subjects: vec![],
                crl_check_enabled_and_no_platform_verifier_and_no_crl_loaded: false,
            });
        }

        let mut verifiers = Vec::<Arc<dyn ServerCertVerifier>>::with_capacity(2);
        let mut root_hint_subjects = Vec::new();

        // WebPkiServerVerifier
        if cert_store.certs.is_empty() {
            if !matches!(crl_check, CrlCheck::None) && !cert_store.crls.is_empty() {
                return Err(SslError::Ssl(
                    "rustls is unable to check certificate revocation with WebPkiServerVerifier but \
                    verify certificates using default platform verifier".to_string(),
                ));
            }
        } else {
            let mut builder = WebPkiServerVerifier::builder_with_provider(
                Arc::new(cert_store.certs.clone()),
                crypto.clone(),
            );
            if !matches!(crl_check, CrlCheck::None) {
                builder = builder.with_crls(cert_store.crls.clone());
                if matches!(crl_check, CrlCheck::Leaf) {
                    builder = builder.only_check_end_entity_revocation();
                }
            }
            let webpki = builder.build().map_err(|e| {
                SslError::Ssl(format!("failed to create WebPkiServerVerifier: {e}"))
            })?;

            root_hint_subjects.extend_from_slice(webpki.root_hint_subjects().unwrap_or(&[]));
            verifiers.push(webpki);
        };

        // Platform verifier.
        if use_system_certificates {
            let platform_verifier =
                rustls_platform_verifier::Verifier::new(crypto).map_err(|e| {
                    SslError::Ssl(format!(
                        "failed to create rustls_platform_verifier::Verifier: {e}"
                    ))
                })?;

            root_hint_subjects
                .extend_from_slice(platform_verifier.root_hint_subjects().unwrap_or(&[]));
            verifiers.push(Arc::new(platform_verifier));
        };

        Ok(Self {
            verify_server_certificates,
            verifiers,
            check_hostname,
            root_hint_subjects,

            crl_check_enabled_and_no_platform_verifier_and_no_crl_loaded: !matches!(
                crl_check,
                CrlCheck::None
            )
                && !use_system_certificates
                && cert_store.crls.is_empty(),
        })
    }

    fn first_server_name<'a>(end_entity: &'a CertificateDer<'a>) -> Option<ServerName<'a>> {
        let (_, cert) = parse_x509_certificate(end_entity.as_ref()).ok()?;
        let san = cert.subject_alternative_name().ok().flatten()?;
        san.value.general_names.iter().find_map(|name| match name {
            GeneralName::DNSName(dns) => DnsName::try_from_str(dns).ok().map(ServerName::DnsName),

            GeneralName::IPAddress(ip) => match ip.len() {
                4 => Some(ServerName::IpAddress(IpAddr::V4(Ipv4Addr::from([
                    ip[0], ip[1], ip[2], ip[3],
                ])))),

                16 => Some(ServerName::IpAddress(IpAddr::V6(
                    Ipv6Addr::from([
                        ip[0], ip[1], ip[2], ip[3], ip[4], ip[5], ip[6], ip[7], ip[8], ip[9],
                        ip[10], ip[11], ip[12], ip[13], ip[14], ip[15],
                    ])
                    .into(),
                ))),

                _ => None,
            },

            _ => None,
        })
    }
}

impl CrlCheck {
    fn from_verify_flags(flags: i32) -> Self {
        if (flags & VERIFY_CRL_CHECK_CHAIN) != 0 {
            Self::Chain
        } else if (flags & VERIFY_CRL_CHECK_LEAF) != 0 {
            Self::Leaf
        } else {
            Self::None
        }
    }
}

#[derive(Debug)]
struct CertStore {
    certs: RootCertStore,
    raw_ca_certs: Vec<Vec<u8>>,
    crls: Vec<CertificateRevocationListDer<'static>>,
    known: HashSet<Vec<u8>>,
    stats: Arc<Stats>,
}

impl CertStore {
    fn empty(stats: Arc<Stats>) -> Self {
        Self {
            certs: RootCertStore::empty(),
            raw_ca_certs: vec![],
            crls: vec![],
            known: HashSet::new(),
            stats,
        }
    }

    fn add_ders(&mut self, ders: &[DerBytes]) {
        for der in ders {
            match der.kind {
                DerKind::Cert => self.add_cert(&der.bytes),
                DerKind::Crl => self.add_crl(&der.bytes),
                DerKind::Key => {} // ignore private keys
            }
        }
    }

    fn add_cert(&mut self, cert: &[u8]) {
        let hash = Self::hash_bytes(cert);
        if self.known.contains(&hash) {
            // Do not add duplicates.
            return;
        }
        let _ = self.known.insert(hash);

        let Ok((_, parsed)) = parse_x509_certificate(cert) else {
            // Silently skip invalid certificates, like OpenSSL does.
            return;
        };

        if parsed.is_ca() || (parsed.subject() == parsed.issuer()) {
            // Add self-signed non-CA (no Basic Constraints) certs too.
            let cert_der = CertificateDer::from_slice(cert);
            if self.certs.add(cert_der).is_ok() {
                let _ = self.stats.cert_store.x509.fetch_add(1, Ordering::Relaxed);

                if parsed.is_ca() || parsed.version().0 == 0 {
                    // Treat self-signed non-CA certs as CA only if version is 0.
                    // This matches cpython/OpenSSL behaviour.
                    self.raw_ca_certs.push(cert.to_vec());
                    let _ = self
                        .stats
                        .cert_store
                        .x509_ca
                        .fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    }

    fn add_crl(&mut self, crl: &[u8]) {
        let hash = Self::hash_bytes(crl);
        if self.known.contains(&hash) {
            // Do not add duplicates.
            return;
        }
        let _ = self.known.insert(hash);

        if parse_x509_crl(crl).is_ok() {
            let crl = CertificateRevocationListDer::from(crl.to_vec());
            self.crls.push(crl);
            let _ = self.stats.cert_store.crl.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn hash_bytes(cert: &[u8]) -> Vec<u8> {
        Sha256::digest(cert).to_vec()
    }

    fn all_certs(&self) -> &[Vec<u8>] {
        &self.raw_ca_certs
    }
}

//
// Stats
//

#[derive(Default, Debug)]
struct Stats {
    cert_store: CertStoreStats,
    session: SessionStats,
}

#[derive(Serialize, Default, Debug)]
struct CertStoreStats {
    #[serde(serialize_with = "serialize_atomic_usize")]
    crl: AtomicUsize,

    #[serde(serialize_with = "serialize_atomic_usize")]
    x509: AtomicUsize,

    #[serde(serialize_with = "serialize_atomic_usize")]
    x509_ca: AtomicUsize,
}

#[derive(Serialize, Default, Debug)]
struct SessionStats {
    #[serde(serialize_with = "serialize_atomic_usize")]
    number: AtomicUsize,

    #[serde(serialize_with = "serialize_atomic_usize")]
    connect: AtomicUsize,

    #[serde(serialize_with = "serialize_atomic_usize")]
    connect_good: AtomicUsize,

    #[serde(serialize_with = "serialize_atomic_usize")]
    connect_renegotiate: AtomicUsize,

    #[serde(serialize_with = "serialize_atomic_usize")]
    accept: AtomicUsize,

    #[serde(serialize_with = "serialize_atomic_usize")]
    accept_good: AtomicUsize,

    #[serde(serialize_with = "serialize_atomic_usize")]
    accept_renegotiate: AtomicUsize,

    #[serde(serialize_with = "serialize_atomic_usize")]
    hits: AtomicUsize,

    #[serde(serialize_with = "serialize_atomic_usize")]
    misses: AtomicUsize,

    #[serde(serialize_with = "serialize_atomic_usize")]
    timeouts: AtomicUsize,

    #[serde(serialize_with = "serialize_atomic_usize")]
    cache_full: AtomicUsize,
}

fn serialize_atomic_usize<S>(atomic: &AtomicUsize, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_u64(atomic.load(Ordering::Relaxed) as u64)
}

//
// OpenSSL cipher string, see `man openssl-ciphers` for details.
//

struct CipherList<'a> {
    ops: Vec<CipherFilterOp<'a>>,
}

enum CipherFilterOp<'a> {
    /// The cipher string @STRENGTH can be used at any point to sort the current cipher list in order of encryption
    /// algorithm key length.
    Strength,

    /// The cipher string @SECLEVEL=n can be used at any point to set the security level to n.
    SecLevel(usize),

    /// Just add matching ciphers to the end of the current list.
    Append(CipherFilterSubOpList<'a>),

    /// If ! is used then the ciphers are permanently deleted from the list. The ciphers deleted can never reappear
    /// in the list even if they are explicitly stated.
    DelAndBlock(CipherFilterSubOpList<'a>),

    /// If - is used then the ciphers are deleted from the list, but some or all of the ciphers can be added again
    /// by later options.
    Del(CipherFilterSubOpList<'a>),

    /// If + is used then the ciphers are moved to the end of the list. This option doesn't add any new ciphers it
    /// just moves matching existing ones.
    MoveToEnd(CipherFilterSubOpList<'a>),
}

struct CipherFilterSubOpList<'a> {
    sub_ops: Vec<CipherFilterSubOp<'a>>,
}

enum CipherFilterSubOp<'a> {
    /// Default cipher list. Valid only as a first operation.
    Default,

    /// The ciphers included in ALL, but not enabled by default.
    ComplementOfDefault,

    /// All cipher suites except the eNULL ciphers.
    All,

    /// The cipher suites not enabled by ALL, currently eNULL.
    ComplementOfAll,

    /// The list of enabled cipher suites will be loaded from the system crypto policy configuration file.
    ProfileSystem,

    /// "High" encryption cipher suites.
    High,

    /// "Medium" encryption cipher suites.
    Medium,

    /// "Low" encryption cipher suites.
    Low,

    /// Lists cipher suites which are only supported in at least TLS v1.0.
    TlsV10,

    /// Lists cipher suites which are only supported in at least TLS v1.2.
    TlsV12,

    /// Lists cipher suites which are only supported in at least SSL v3.
    SslV3,

    /// Enables suite B mode of operation.
    SuiteB(SuiteBType),

    /// All cipher suites using encryption algorithm in Cipher Block Chaining (CBC) mode.
    Cbc,

    /// AES in Galois Counter Mode (GCM): these cipher suites are only supported in TLS v1.2.
    AesGcm,

    /// Match by message authentication algorithm.
    Auth(&'a str),

    /// Match by key exchange algorithm.
    KeyEx(&'a str),

    /// Match by part of an OpenSSL name that usually contains key exchange algorithm and symmetric cipher
    /// and may contain other identifiers.
    Part(&'a str),

    /// Match by full OpenSSL or IANA cipher name.
    Full(&'a str),
}

enum SuiteBType {
    Use128Permit192,
    Use128Only,
    Use192Only,
}

impl<'a> CipherList<'a> {
    fn parse_to_rustls(
        s: &'a str,
    ) -> Result<WithOptionSuiteB<Vec<SupportedCipherSuite>>, &'static str> {
        Self::parse(s)?.to_rustls()
    }

    fn parse(s: &'a str) -> Result<Self, &'static str> {
        let ops: Vec<_> = s
            .split(|c: char| c == ':' || c == ',' || c.is_ascii_whitespace())
            .filter(|s| !s.is_empty())
            .enumerate()
            .map(|(i, s)| {
                let suite_b = match s {
                    "SUITEB128" => Some(CipherFilterSubOp::SuiteB(SuiteBType::Use128Permit192)),
                    "SUITEB128ONLY" => Some(CipherFilterSubOp::SuiteB(SuiteBType::Use128Only)),
                    "SUITEB192" => Some(CipherFilterSubOp::SuiteB(SuiteBType::Use192Only)),
                    _ => None,
                };

                match (i, s, suite_b) {
                    (0, "DEFAULT", _) => Ok(CipherFilterOp::Append(
                        CipherFilterSubOpList::from_sub_op(CipherFilterSubOp::Default),
                    )),

                    (0, _, Some(suite_b)) => Ok(CipherFilterOp::Append(
                        CipherFilterSubOpList::from_sub_op(suite_b),
                    )),

                    (_, _, _) => CipherFilterOp::parse(s),
                }
            })
            .collect::<Result<_, _>>()?;
        if ops.is_empty() {
            Err("list of ciphers is empty")
        } else {
            Ok(Self { ops })
        }
    }

    fn to_rustls(&self) -> Result<WithOptionSuiteB<Vec<SupportedCipherSuite>>, &'static str> {
        let mut min_bits = SECURITY_LEVEL_TO_MIN_BITS[0];
        let mut block_list = Vec::new();
        let mut ids = Vec::new();

        let sanitize = |ids: &mut Vec<u16>, min_bits, block_list: &[u16]| {
            ids.retain(|id| CIPHER_MAPPINGS.id_to_bits[id] >= min_bits);
            ids.retain(|id| !block_list.contains(id));
        };
        let extend = |ids: &mut Vec<u16>, source: &[u16]| {
            // Extend and deduplicate.
            for id in source {
                if !ids.contains(id) {
                    ids.push(*id);
                }
            }
        };
        let ids_to_suits = |ids: &[u16]| {
            ids.iter()
                .map(|id| *CIPHER_MAPPINGS.id_to_cipher[id])
                .collect()
        };

        for op in &self.ops {
            match op {
                CipherFilterOp::Strength => {
                    ids.sort_by_key(|id| -i32::from(CIPHER_MAPPINGS.id_to_bits[id]))
                }

                CipherFilterOp::SecLevel(level) => {
                    min_bits = *SECURITY_LEVEL_TO_MIN_BITS
                        .get(*level)
                        .ok_or("@SECLEVEL value too big")?;
                    sanitize(&mut ids, min_bits, &block_list);
                }

                CipherFilterOp::Append(sub_op_list) => {
                    let (mut new_ids, suite_b) = sub_op_list.to_rustls_ids()?;
                    if suite_b.is_some() {
                        // SUITEB* cipherstrings should appear first in the cipher list and anything
                        // after them is ignored.
                        return Ok((ids_to_suits(&new_ids), suite_b));
                    }
                    sanitize(&mut new_ids, min_bits, &block_list);
                    extend(&mut ids, &new_ids);
                }

                CipherFilterOp::DelAndBlock(sub_op_list) => {
                    extend(&mut block_list, &sub_op_list.to_rustls_ids()?.0);
                    sanitize(&mut ids, min_bits, &block_list);
                }

                CipherFilterOp::Del(sub_op_list) => {
                    let (del_ids, _) = sub_op_list.to_rustls_ids()?;
                    ids.retain(|id| !del_ids.contains(id));
                }

                CipherFilterOp::MoveToEnd(sub_op_list) => {
                    let (move_ids, _) = sub_op_list.to_rustls_ids()?;
                    ids.sort_by_key(|id| move_ids.contains(id))
                }
            }
        }

        Ok((ids_to_suits(&ids), None))
    }
}

impl<'a> CipherFilterOp<'a> {
    fn parse(mut s: &'a str) -> Result<Self, &'static str> {
        if s == "@STRENGTH" {
            return Ok(Self::Strength);
        }
        const SECLEVEL: &str = "@SECLEVEL=";
        if s.starts_with(SECLEVEL) {
            return Ok(Self::SecLevel(
                usize::from_str(s.get(SECLEVEL.len()..).unwrap_or(""))
                    .map_err(|_| "invalid @SECLEVEL value")?,
            ));
        }

        let prefix = s.get(..1).unwrap_or("");
        if ["!", "-", "+"].contains(&prefix) {
            s = s.get(1..).unwrap_or("");
        }
        Ok(match prefix {
            "!" => Self::DelAndBlock(CipherFilterSubOpList::parse(s)?),
            "-" => Self::Del(CipherFilterSubOpList::parse(s)?),
            "+" => Self::MoveToEnd(CipherFilterSubOpList::parse(s)?),
            _ => Self::Append(CipherFilterSubOpList::parse(s)?),
        })
    }
}

impl<'a> CipherFilterSubOpList<'a> {
    fn parse(s: &'a str) -> Result<Self, &'static str> {
        let sub_ops: Vec<_> = s
            .split('+')
            .filter(|s| !s.is_empty())
            .map(CipherFilterSubOp::parse)
            .collect::<Result<_, _>>()?;
        if sub_ops.is_empty() {
            Err("list of cipher filtering operations is empty")
        } else {
            Ok(Self { sub_ops })
        }
    }

    fn from_sub_op(sub_op: CipherFilterSubOp<'a>) -> Self {
        Self {
            sub_ops: vec![sub_op],
        }
    }

    fn to_rustls_ids(&self) -> Result<WithOptionSuiteB<Vec<u16>>, &'static str> {
        let mut ids = Vec::new();
        for sub_op in &self.sub_ops {
            match sub_op {
                CipherFilterSubOp::Default => {
                    Self::extend_or_intersect(&mut ids, &CIPHER_MAPPINGS.default)
                }

                CipherFilterSubOp::ComplementOfDefault => {
                    Self::extend_or_intersect(&mut ids, &CIPHER_MAPPINGS.complement_of_default)
                }

                CipherFilterSubOp::All => Self::extend_or_intersect(&mut ids, &CIPHER_MAPPINGS.all),

                CipherFilterSubOp::ComplementOfAll => {
                    Self::extend_or_intersect(&mut ids, &CIPHER_MAPPINGS.complement_of_all)
                }

                CipherFilterSubOp::ProfileSystem => {
                    return Err(
                        "reading cipher suites from system crypto policy file is not supported with rustls",
                    );
                }

                // Here we trust that all default rustls cipher suites can be considered "high".
                CipherFilterSubOp::High => {
                    Self::extend_or_intersect(&mut ids, &CIPHER_MAPPINGS.default)
                }
                CipherFilterSubOp::Medium => {
                    Self::extend_or_intersect(&mut ids, &CIPHER_MAPPINGS.default)
                }
                CipherFilterSubOp::Low => {
                    Self::extend_or_intersect(&mut ids, &CIPHER_MAPPINGS.default)
                }

                CipherFilterSubOp::TlsV10 | CipherFilterSubOp::SslV3 => {} // rustls does not support older ciphers

                CipherFilterSubOp::TlsV12 => {
                    Self::extend_or_intersect(&mut ids, &CIPHER_MAPPINGS.tls_1_2)
                }

                // RFC 6460
                CipherFilterSubOp::SuiteB(SuiteBType::Use128Permit192) => {
                    return Ok((
                        vec![
                            CipherSuite::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256.into(),
                            CipherSuite::TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384.into(),
                        ],
                        Some(vec![
                            kx_group_by_name(rustls::NamedGroup::secp256r1, "secp256r1")?,
                            kx_group_by_name(rustls::NamedGroup::secp384r1, "secp384r1")?,
                        ]),
                    ));
                }
                CipherFilterSubOp::SuiteB(SuiteBType::Use128Only) => {
                    return Ok((
                        vec![CipherSuite::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256.into()],
                        Some(vec![kx_group_by_name(
                            rustls::NamedGroup::secp256r1,
                            "secp256r1",
                        )?]),
                    ));
                }
                CipherFilterSubOp::SuiteB(SuiteBType::Use192Only) => {
                    return Ok((
                        vec![CipherSuite::TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384.into()],
                        Some(vec![kx_group_by_name(
                            rustls::NamedGroup::secp384r1,
                            "secp384r1",
                        )?]),
                    ));
                }

                CipherFilterSubOp::Cbc => {
                    let mut rhs = Vec::with_capacity(CIPHER_MAPPINGS.iana_to_id.len());

                    // OpenSSL names might contain either -CBC- or -CBC3-, IANA seems to only contain _CBC_.
                    for (iana, rustls_id) in &CIPHER_MAPPINGS.iana_to_id {
                        if iana.split('_').any(|s| s == "CBC") {
                            rhs.push(*rustls_id)
                        }
                    }

                    Self::extend_or_intersect(&mut ids, &rhs)
                }

                CipherFilterSubOp::AesGcm => {
                    let mut rhs = Vec::with_capacity(CIPHER_MAPPINGS.iana_to_id.len());

                    for (openssl, rustls_id) in &CIPHER_MAPPINGS.openssl_to_id {
                        if openssl.split(['-', '_']).any(|s| s.starts_with("AES"))
                            && openssl.split(['-', '_']).any(|s| s == "GCM")
                        {
                            rhs.push(*rustls_id)
                        }
                    }

                    Self::extend_or_intersect(&mut ids, &rhs)
                }

                CipherFilterSubOp::Auth(auth) => {
                    let rhs: Vec<_> = CIPHER_MAPPINGS
                        .id_to_cipher
                        .iter()
                        .filter_map(|(k, v)| {
                            match v {
                                SupportedCipherSuite::Tls12(c) => {
                                    let mut maybe_id = None;
                                    for scheme in c.sign {
                                        if scheme
                                            .as_str()
                                            .is_some_and(|s| s.split('_').any(|s| s == *auth))
                                        {
                                            maybe_id = Some(*k);
                                        }
                                    }
                                    maybe_id
                                }

                                // usable_for_signature_algorithm() always returns true for TLS 1.3.
                                SupportedCipherSuite::Tls13(_) => Some(*k),
                            }
                        })
                        .collect();
                    Self::extend_or_intersect(&mut ids, &rhs)
                }

                CipherFilterSubOp::KeyEx(key_ex) => {
                    let rhs: Vec<_> = CIPHER_MAPPINGS
                        .id_to_key_ex
                        .iter()
                        .filter_map(|(k, v)| if v == key_ex { Some(*k) } else { None })
                        .collect();
                    Self::extend_or_intersect(&mut ids, &rhs)
                }

                CipherFilterSubOp::Part(part) => {
                    let mut rhs = Vec::with_capacity(CIPHER_MAPPINGS.iana_to_id.len());

                    for (openssl, rustls_id) in &CIPHER_MAPPINGS.openssl_to_id {
                        if openssl.split(['-', '_']).any(|s| &s == part) {
                            rhs.push(*rustls_id)
                        }
                    }

                    Self::extend_or_intersect(&mut ids, &rhs)
                }

                CipherFilterSubOp::Full(full) => {
                    if let Some(id) = CIPHER_MAPPINGS
                        .openssl_to_id
                        .get(full)
                        .or_else(|| CIPHER_MAPPINGS.iana_to_id.get(full))
                    {
                        Self::extend_or_intersect(&mut ids, &[*id])
                    }
                }
            }
        }
        Ok((ids, None))
    }

    fn extend_or_intersect(lhs: &mut Vec<u16>, rhs: &[u16]) {
        if lhs.is_empty() {
            lhs.extend_from_slice(rhs)
        } else {
            lhs.retain(|id| rhs.contains(id))
        }
    }
}

fn kx_group_by_name(
    name: rustls::NamedGroup,
    error_name: &'static str,
) -> Result<&'static dyn SupportedKxGroup, &'static str> {
    CryptoExt::get_ext()
        .all_kx_or_default()
        .iter()
        .find(|g| g.name() == name)
        .copied()
        .ok_or(error_name)
}

type WithOptionSuiteB<T> = (T, Option<Vec<&'static dyn SupportedKxGroup>>);

impl<'a> CipherFilterSubOp<'a> {
    fn parse(mut s: &'a str) -> Result<Self, &'static str> {
        Ok(match s {
            "DEFAULT" => return Err("DEFAULT specified at wrong position in the cipher string"),
            "SUITEB128" => {
                return Err("SUITEB128 specified at wrong position in the cipher string");
            }
            "SUITEB128ONLY" => {
                return Err("SUITEB128ONLY specified at wrong position in the cipher string");
            }
            "SUITEB192" => {
                return Err("SUITEB192 specified at wrong position in the cipher string");
            }

            "COMPLEMENTOFDEFAULT" => Self::ComplementOfDefault,
            "ALL" => Self::All,
            "COMPLEMENTOFALL" => Self::ComplementOfAll,
            "PROFILE=SYSTEM" => Self::ProfileSystem,
            "HIGH" => Self::High,
            "MEDIUM" => Self::Medium,
            "LOW" => Self::Low,
            "TLSv1.0" => Self::TlsV10,
            "TLSv1.2" => Self::TlsV12,
            "SSLv3" => Self::SslV3,
            "CBC" => Self::Cbc,
            "AESGCM" => Self::AesGcm,

            // RSA is an alias for kRSA.
            "RSA" => Self::KeyEx("RSA"),

            _ => {
                let prefix = s.get(..1).unwrap_or("");
                if ["a", "k", "e"].contains(&prefix) {
                    s = s.get(1..).unwrap_or("");
                }

                if s.is_empty() {
                    return Err("item of cipher string is empty");
                }
                if !s
                    .chars()
                    .all(|c| char::is_ascii_alphanumeric(&c) || matches!(c, '-' | '_'))
                {
                    return Err("item of cipher string contains invalid characters");
                }

                match prefix {
                    "a" => Self::Auth(s),
                    "k" => Self::KeyEx(s),
                    "e" => Self::Part(s),

                    _ => {
                        if s.contains(['_', '-']) {
                            Self::Full(s)
                        } else {
                            Self::Part(s)
                        }
                    }
                }
            }
        })
    }
}

static CIPHER_MAPPINGS: LazyLock<CipherMappings> = LazyLock::new(CipherMappings::new);

struct CipherMappings {
    complement_of_default: Vec<u16>,
    complement_of_all: Vec<u16>,
    default: Vec<u16>,
    all: Vec<u16>,
    tls_1_2: Vec<u16>,
    // TODO: Consolidate id_to_* into single HashMap.
    id_to_openssl: HashMap<u16, &'static str>,
    id_to_key_ex: HashMap<u16, &'static str>,
    id_to_bits: HashMap<u16, u16>,
    id_to_cipher: HashMap<u16, &'static SupportedCipherSuite>,
    openssl_to_id: HashMap<&'static str, u16>,
    iana_to_id: HashMap<&'static str, u16>,

    name_to_kx_group: HashMap<String, &'static dyn SupportedKxGroup>,
}

impl CipherMappings {
    fn new() -> Self {
        let all_cipher_suites = CryptoExt::get_ext().all_ciphers_or_default();
        let default_cipher_suites = CryptoExt::get_ext().default_ciphers_or_provider();

        let mut all = Vec::with_capacity(all_cipher_suites.len());
        let mut tls_1_2 = Vec::with_capacity(all_cipher_suites.len());
        let mut id_to_openssl = HashMap::with_capacity(all_cipher_suites.len());
        let mut id_to_key_ex = HashMap::with_capacity(all_cipher_suites.len());
        let mut id_to_bits = HashMap::with_capacity(all_cipher_suites.len());
        let mut id_to_cipher = HashMap::with_capacity(all_cipher_suites.len());
        let mut openssl_to_id = HashMap::with_capacity(all_cipher_suites.len());
        let mut iana_to_id = HashMap::with_capacity(all_cipher_suites.len());

        for cipher in all_cipher_suites {
            // See https://www.ssl.org/cipher-suite-mapping
            let (openssl, iana, key_ex, bits, min_tls_ver) = match cipher.suite() {
                CipherSuite::TLS13_AES_256_GCM_SHA384 => (
                    "TLS_AES_256_GCM_SHA384",
                    "TLS_AES_256_GCM_SHA384",
                    "ECDH",
                    256,
                    13,
                ),

                CipherSuite::TLS13_AES_128_GCM_SHA256 => (
                    "TLS_AES_128_GCM_SHA256",
                    "TLS_AES_128_GCM_SHA256",
                    "ECDH",
                    128,
                    13,
                ),

                CipherSuite::TLS13_CHACHA20_POLY1305_SHA256 => (
                    "TLS_CHACHA20_POLY1305_SHA256",
                    "TLS_CHACHA20_POLY1305_SHA256",
                    "ECDH",
                    256,
                    13,
                ),

                CipherSuite::TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384 => (
                    "ECDHE-ECDSA-AES256-GCM-SHA384",
                    "TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384",
                    "ECDH",
                    256,
                    12,
                ),

                CipherSuite::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256 => (
                    "ECDHE-ECDSA-AES128-GCM-SHA256",
                    "TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256",
                    "ECDH",
                    128,
                    12,
                ),

                CipherSuite::TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256 => (
                    "ECDHE-ECDSA-CHACHA20-POLY1305",
                    "TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256",
                    "ECDH",
                    256,
                    12,
                ),

                CipherSuite::TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384 => (
                    "ECDHE-RSA-AES256-GCM-SHA384",
                    "TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384",
                    "ECDH",
                    256,
                    12,
                ),

                CipherSuite::TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256 => (
                    "ECDHE-RSA-AES128-GCM-SHA256",
                    "TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256",
                    "ECDH",
                    128,
                    12,
                ),

                CipherSuite::TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256 => (
                    "ECDHE-RSA-CHACHA20-POLY1305",
                    "TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256",
                    "ECDH",
                    256,
                    12,
                ),

                // This is tested by that_all_rustls_ciphers_are_known().
                // This may happen after rustls update, just add more ciphers above is this case.
                _ => unreachable!("BUG: Unknown cipher suite {cipher:?}"),
            };

            let id = cipher.suite().into();

            if bits > 0 {
                all.push(id);
            }
            if min_tls_ver >= 12 {
                tls_1_2.push(id);
            }
            let _ = id_to_openssl.insert(id, openssl);
            let _ = id_to_key_ex.insert(id, key_ex);
            let _ = id_to_bits.insert(id, bits);
            let _ = id_to_cipher.insert(id, cipher);
            let _ = openssl_to_id.insert(openssl, id);
            let _ = iana_to_id.insert(iana, id);
        }

        let default: Vec<_> = default_cipher_suites
            .iter()
            .map(|c| u16::from(c.suite()))
            .collect();

        Self {
            complement_of_default: all_cipher_suites
                .iter()
                .filter(|c| !default.contains(&c.suite().into()))
                .map(|c| u16::from(c.suite()))
                .collect(),
            complement_of_all: all_cipher_suites
                .iter()
                .filter(|c| !all.contains(&c.suite().into()))
                .map(|c| u16::from(c.suite()))
                .collect(),

            default,
            all,
            tls_1_2,
            id_to_openssl,
            id_to_key_ex,
            id_to_bits,
            id_to_cipher,
            openssl_to_id,
            iana_to_id,

            name_to_kx_group: CryptoExt::get_ext()
                .all_kx_or_default()
                .iter()
                .map(|g| (kx_group_openssl_name(*g).to_owned(), *g))
                .collect(),
        }
    }
}

fn kx_group_openssl_name(group: &dyn SupportedKxGroup) -> &'static str {
    match group.name() {
        rustls::NamedGroup::secp256r1 => "prime256v1",
        rustls::NamedGroup::secp384r1 => "secp384r1",
        rustls::NamedGroup::X25519 => "X25519",
        rustls::NamedGroup::MLKEM768 => "MLKEM768",
        rustls::NamedGroup::MLKEM1024 => "MLKEM1024",
        rustls::NamedGroup::secp256r1MLKEM768 => "SecP256r1MLKEM768",
        rustls::NamedGroup::X25519MLKEM768 => "X25519MLKEM768",

        // This is tested by that_all_rustls_kx_groups_have_openssl_names()
        name => unreachable!("BUG: Unknown key exchange group {name:?}"),
    }
}

// See `man SSL_CTX_set_security_level` for details.
const SECURITY_LEVEL_TO_MIN_BITS: &[u16] = &[0, 80, 112, 128, 192, 256];

//
// Oid registry for txt2obj() and nid2obj()
//

static OID_MAPPINGS: LazyLock<OidMappings> = LazyLock::new(OidMappings::new);

struct OidMappings {
    name_to_oid: HashMap<&'static str, Oid<'static>>,
    oid_to_entry: OidRegistry<'static>,
    oid_sn_to_nid: HashMap<(Oid<'static>, &'static str), u16>,
    nid_to_oid: HashMap<u16, Oid<'static>>,
}

impl OidMappings {
    fn new() -> Self {
        let mut name_to_oid = HashMap::new();
        let mut oid_to_entry = OidRegistry::default();
        let mut oid_sn_to_nid = HashMap::new();
        let mut nid_to_oid = HashMap::new();

        // See https://github.com/openssl/openssl/blob/11b7b6ea3b65a584e1d31408ed1bdb139465cffd/crypto/objects/README.md
        // See https://github.com/openssl/openssl/blob/11b7b6ea3b65a584e1d31408ed1bdb139465cffd/crypto/objects/objects.pl
        // TODO: Do this in compile time.
        let obj_mac_num = include_str!("rustls-data/obj_mac.num");
        let objects_txt = include_str!("rustls-data/objects.txt");

        let nids: HashMap<_, _> = obj_mac_num
            .split('\n')
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::split_whitespace)
            .map(|mut line| {
                let name = line.next().expect("BUG: Impossible");
                let nid = line
                    .next()
                    .expect("BUG: No NID")
                    .parse()
                    .expect("BUG: Invalid NID");
                (name, nid)
            })
            .collect();

        let mut aliases: HashMap<Rc<String>, Rc<Vec<u64>>> = HashMap::new();
        let mut cname = None;
        let mut module: Option<&'static str> = None;
        for line in objects_txt
            .split('\n')
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .filter(|l| !l.starts_with('#'))
        {
            let prepend_module = |s: &str| {
                if let Some(module) = module {
                    let mut str_buf = String::with_capacity(module.len() + 1 + s.len());
                    str_buf.push_str(module);
                    str_buf.push('_');
                    str_buf.push_str(s);
                    Rc::new(str_buf.replace('-', "_"))
                } else {
                    Rc::new(s.replace('-', "_"))
                }
            };

            if line.starts_with('!') {
                let mut splitted = line.split_whitespace();
                match splitted.next().expect("BUG: Impossible") {
                    "!Alias" => {
                        let alias = splitted.next().expect("BUG: No alias after !Alias");

                        let mut oid = Vec::with_capacity(16);
                        for oid_part in splitted {
                            // Resolve aliases to make sure that they are not recursive.
                            if let Some(oid_part) = aliases.get(&oid_part.replace('-', "_")) {
                                oid.extend_from_slice(oid_part);
                            } else {
                                oid.push(
                                    u64::from_str(oid_part)
                                        .expect("BUG: OID part in alias can not be parsed as u64"),
                                );
                            }
                        }
                        assert!(!oid.is_empty(), "BUG: Empty OID for alias {alias}");
                        let res = aliases.insert(prepend_module(alias), Rc::new(oid));
                        assert!(res.is_none(), "BUG: Duplicate alias {alias}");
                    }

                    "!Cname" => {
                        assert!(cname.is_none(), "BUG: Double !Cname");
                        cname = Some(splitted.next().expect("BUG: No name after !Cname"));
                        assert!(
                            splitted.next().is_none(),
                            "BUG: Extra elements after !Cname"
                        );
                    }

                    "!module" => {
                        assert!(module.is_none(), "BUG: Double !module");
                        module = Some(splitted.next().expect("BUG: No name after !module"));
                        assert!(
                            splitted.next().is_none(),
                            "BUG: Extra elements after !module"
                        );
                    }

                    "!global" => {
                        assert!(module.is_some(), "BUG: !global without !module");
                        module = None;
                        assert!(
                            splitted.next().is_none(),
                            "BUG: Extra elements after !global"
                        );
                    }

                    cmd => panic!("BUG: Unknown objects.txt command: {cmd}"),
                }
                continue;
            }

            // OID string
            let mut line = line.split(':').map(|s| s.trim());
            let oid_str = line.next().expect("BUG: No OID");
            let mut oid = Vec::with_capacity(16);
            for oid_part in oid_str.split_whitespace() {
                if let Some(oid_part) = aliases.get(&oid_part.replace('-', "_")) {
                    oid.extend_from_slice(oid_part);
                } else {
                    oid.push(
                        u64::from_str(oid_part).expect("BUG: OID part can not be parsed as u64"),
                    );
                }
            }
            let oid = Rc::new(oid);

            // Short name and description
            let sn = line.next().expect("BUG: No SN");
            let desc = line.next();
            if desc.is_some() {
                assert!(
                    line.next().is_none(),
                    "BUG: Extra elements after OID, SN and description"
                );
            }
            let desc = desc.unwrap_or("");

            // Add into aliases.
            let mut added_now = Vec::with_capacity(3);
            if let Some(cname) = cname.take() {
                let owned_cname = prepend_module(cname);
                added_now.push(owned_cname.clone());
                let res = aliases.insert(owned_cname.clone(), oid.clone());
                assert!(res.is_none(), "BUG: Duplicate cname {owned_cname}");
            };
            if !desc.is_empty() {
                let owned_desc = prepend_module(desc);
                if !added_now.contains(&owned_desc) {
                    added_now.push(owned_desc.clone());
                    let res = aliases.insert(owned_desc.clone(), oid.clone());
                    assert!(res.is_none(), "BUG: Duplicate description {owned_desc}");
                }
            };
            if !sn.is_empty() {
                let owned_sn = prepend_module(sn);
                if !added_now.contains(&owned_sn) {
                    added_now.push(owned_sn.clone());
                    let res = aliases.insert(owned_sn.clone(), oid.clone());
                    assert!(res.is_none(), "BUG: Duplicate SN {owned_sn}");
                }
            }

            if matches!(oid.as_slice(), [] | [1 | 2]) {
                // Can not be added into registry.
                continue;
            }
            let owned_oid = Oid::from(&oid).expect("BUG: Invalid OID array");
            if !sn.is_empty() {
                let res = name_to_oid.insert(sn, owned_oid.clone());
                assert!(
                    res.is_none(),
                    "BUG: Duplicate SN -> OID mapping: {sn} -> {owned_oid}"
                );
            }
            if !desc.is_empty() && desc != sn {
                let res = name_to_oid.insert(desc, owned_oid.clone());
                assert!(
                    res.is_none(),
                    "BUG: Duplicate Description -> OID mapping: {sn} -> {owned_oid}"
                );
            }
            // Allow some duplicated OIDs.
            if oid_to_entry
                .insert(owned_oid.clone(), OidEntry::new(sn, desc))
                .is_some()
                && !matches!(oid.as_slice(), [1, 3])
            {
                panic!("BUG: Duplicate OID: {oid:?}");
            }

            for added in &added_now {
                if let Some(nid) = nids.get(added.as_str()) {
                    let res = oid_sn_to_nid.insert((owned_oid.clone(), sn), *nid);
                    assert!(
                        res.is_none(),
                        "BUG: Duplicate (OID, SN) -> NID mapping: ({owned_oid}, {sn}) -> {nid}"
                    );
                    break;
                }
            }
            for added in &added_now {
                if let Some(nid) = nids.get(added.as_str()) {
                    let res = nid_to_oid.insert(*nid, owned_oid.clone());
                    assert!(
                        res.is_none(),
                        "BUG: Duplicate NID -> OID mapping: {nid} -> {owned_oid}"
                    );
                }
            }
        }
        assert!(cname.is_none(), "BUG: Unused !Cname");
        assert!(module.is_none(), "BUG: !module not closed");

        Self {
            name_to_oid,
            oid_to_entry,
            oid_sn_to_nid,
            nid_to_oid,
        }
    }
}

// TODO: Test with different providers.
#[cfg(test)]
mod tests {
    use core::hint::black_box;

    use std::sync::Once;

    use rustls::crypto::aws_lc_rs;

    use super::*;

    #[test]
    fn that_all_rustls_tls_versions_are_known() {
        install_test_crypto_provider();
        for cipher in CryptoExt::get_ext().all_ciphers_or_default() {
            let _ = black_box(CipherDescriptionDict::new(cipher));
        }
    }

    #[test]
    fn that_all_rustls_ciphers_are_known() {
        install_test_crypto_provider();
        let _ = black_box(&CIPHER_MAPPINGS.id_to_openssl);
    }

    #[test]
    fn that_all_rustls_kx_groups_have_openssl_names() {
        install_test_crypto_provider();
        let _ = black_box(&CIPHER_MAPPINGS.name_to_kx_group);
    }

    #[test]
    fn cipher_list_default_and_names() {
        install_test_crypto_provider();

        let default = CryptoExt::get_ext()
            .default_ciphers_or_provider()
            .iter()
            .map(|suite| suite.suite())
            .collect::<Vec<_>>();
        let (suites, suite_b) = CipherList::parse_to_rustls("DEFAULT").unwrap();

        assert!(suite_b.is_none());
        assert_eq!(
            suites.iter().map(|suite| suite.suite()).collect::<Vec<_>>(),
            default
        );
        assert_eq!(
            cipher_names("TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256, ECDHE-ECDSA-AES128-GCM-SHA256"),
            [
                "ECDHE-RSA-AES128-GCM-SHA256",
                "ECDHE-ECDSA-AES128-GCM-SHA256",
            ]
        );
        assert_eq!(
            cipher_names("AES128+aECDSA"),
            ["ECDHE-ECDSA-AES128-GCM-SHA256"]
        );
    }

    #[test]
    fn cipher_list_deletes_and_moves() {
        install_test_crypto_provider();

        assert_eq!(
            cipher_names(
                "ECDHE-ECDSA-AES128-GCM-SHA256:ECDHE-RSA-AES128-GCM-SHA256:+ECDHE-ECDSA-AES128-GCM-SHA256"
            ),
            [
                "ECDHE-RSA-AES128-GCM-SHA256",
                "ECDHE-ECDSA-AES128-GCM-SHA256",
            ]
        );
        assert_eq!(
            cipher_names(
                "ECDHE-RSA-AES128-GCM-SHA256:ECDHE-ECDSA-AES128-GCM-SHA256:-ECDHE-ECDSA-AES128-GCM-SHA256:ECDHE-ECDSA-AES128-GCM-SHA256:!ECDHE-RSA-AES128-GCM-SHA256:ECDHE-RSA-AES128-GCM-SHA256"
            ),
            ["ECDHE-ECDSA-AES128-GCM-SHA256"]
        );
    }

    #[test]
    fn cipher_list_strength_and_security_level() {
        install_test_crypto_provider();

        assert_eq!(
            cipher_names("ECDHE-RSA-AES128-GCM-SHA256:ECDHE-RSA-AES256-GCM-SHA384:@STRENGTH"),
            ["ECDHE-RSA-AES256-GCM-SHA384", "ECDHE-RSA-AES128-GCM-SHA256"]
        );
        assert_eq!(
            cipher_names("ECDHE-RSA-AES128-GCM-SHA256:@SECLEVEL=4:ECDHE-RSA-AES256-GCM-SHA384"),
            ["ECDHE-RSA-AES256-GCM-SHA384"]
        );
    }

    #[test]
    fn cipher_list_suite_b() {
        install_test_crypto_provider();

        let (suites, suite_b) = CipherList::parse_to_rustls("SUITEB128:ALL").unwrap();

        assert_eq!(
            suites.iter().map(|suite| suite.suite()).collect::<Vec<_>>(),
            [
                CipherSuite::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256,
                CipherSuite::TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384,
            ]
        );
        assert!(suite_b.is_some());
        assert!(CipherList::parse_to_rustls("ALL:SUITEB128").is_err());
    }

    #[test]
    fn cipher_list_errors() {
        install_test_crypto_provider();

        assert!(CipherList::parse_to_rustls("ALL:DEFAULT").is_err());
        assert!(CipherList::parse_to_rustls("ALL:@SECLEVEL=6").is_err());
        assert!(CipherList::parse_to_rustls("PROFILE=SYSTEM").is_err());
        assert!(CipherList::parse_to_rustls(";").is_err());
        assert!(CipherList::parse_to_rustls("").is_err());
    }

    fn cipher_names(s: &str) -> Vec<&'static str> {
        install_test_crypto_provider();

        let (suites, suite_b) = CipherList::parse_to_rustls(s).unwrap();
        assert!(suite_b.is_none());
        suites
            .iter()
            .map(|suite| {
                let id: u16 = suite.suite().into();
                CIPHER_MAPPINGS.id_to_openssl[&id]
            })
            .collect()
    }

    fn install_test_crypto_provider() {
        static ONCE: Once = Once::new();
        ONCE.call_once(|| {
            let ext = CryptoExt {
                all_cipher_suites: Some(aws_lc_rs::ALL_CIPHER_SUITES),
                default_cipher_suites: Some(aws_lc_rs::DEFAULT_CIPHER_SUITES),
                all_kx_groups: Some(aws_lc_rs::ALL_KX_GROUPS),
                any_supported_key: Some(aws_lc_rs::sign::any_supported_type),
                ticketer: aws_lc_rs::Ticketer::new,
            };
            CryptoExt::set_provider(aws_lc_rs::default_provider(), ext).unwrap();
        })
    }

    #[test]
    fn oid_mappings() {
        let _ = black_box(&OID_MAPPINGS.name_to_oid);
        let _ = black_box(&OID_MAPPINGS.oid_to_entry);
        let _ = black_box(&OID_MAPPINGS.oid_sn_to_nid);
        let _ = black_box(&OID_MAPPINGS.name_to_oid);
    }
}
