// spell-checker: ignore ssleof aesccm aesgcm capath getblocking setblocking ENDTLS TLSEXT

//! Pure Rust SSL/TLS implementation using rustls
//!
//! This module provides SSL/TLS support without requiring C dependencies.
//! It implements the Python ssl module API using:
//! - rustls: TLS protocol implementation
//! - x509-parser/x509-cert: Certificate parsing
//! - ring: Cryptographic primitives
//! - rustls-platform-verifier: Platform-native certificate verification
//!
//! DO NOT add openssl dependency here.
//!
//! Warning: This library contains AI-generated code and comments. Do not trust any code or comment without verification. Please have a qualified expert review the code and remove this notice after review.

// OID (Object Identifier) management module
mod oid;

// Certificate operations module (parsing, validation, conversion)
mod cert;

mod chain;

/// OpenSSL cipher string parsing for `set_ciphers`
mod cipher;

// OpenSSL compatibility layer (abstracts rustls operations)
mod compat;

// SSL exception types (shared with openssl backend)
mod error;

// Utilities for setting a Rustls cryptography provider.
pub mod providers;

pub(crate) use _ssl::module_def;

#[allow(non_snake_case)]
#[allow(non_upper_case_globals)]
#[pymodule(with(error::ssl_error))]
mod _ssl {
    use crate::{
        common::{
            hash::PyHash,
            lock::{PyMutex, PyRwLock},
        },
        socket::{PySocket, SockWaitKind, sock_wait, timeout_error_msg},
        vm::{
            AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject,
            VirtualMachine,
            builtins::{
                PyBaseExceptionRef, PyByteArray, PyBytesRef, PyListRef, PyStrRef, PyType,
                PyTypeRef, PyUtf8StrRef, PyWeak,
            },
            convert::IntoPyException,
            function::{
                ArgBytesLike, ArgMemoryBuffer, Either, FuncArgs, OptionalArg, PyComparisonValue,
            },
            stdlib::_warnings,
            types::{Comparable, Constructor, Hashable, PyComparisonOp, Representable},
        },
    };

    // Import error types used in this module (others are exposed via pymodule(with(...)))
    use super::error::{
        PySSLError, create_ssl_eof_error, create_ssl_want_read_error, create_ssl_want_write_error,
        create_ssl_zero_return_error,
    };
    use alloc::sync::Arc;
    use core::{
        hash::{Hash, Hasher},
        hint::cold_path,
        sync::atomic::{AtomicUsize, Ordering},
        time::Duration,
    };
    use memchr::memchr;
    use rustpython_vm::exceptions;
    use std::{
        collections::{HashMap, hash_map::DefaultHasher},
        io::BufRead,
        time::SystemTime,
    };

    // Rustls imports
    use parking_lot::{Mutex as ParkingMutex, RwLock as ParkingRwLock};
    use pem_rfc7468::{LineEnding, encode_string};
    use rustls::{
        ClientConnection, Connection, HandshakeKind, RootCertStore, ServerConnection,
        client::{ClientSessionMemoryCache, ClientSessionStore},
        crypto::SupportedKxGroup,
        pki_types::{CertificateDer, CertificateRevocationListDer, PrivateKeyDer, ServerName},
        server::{ClientHello, ResolvesServerCert},
        sign::CertifiedKey,
        version::{TLS12, TLS13},
    };
    use sha2::{Digest, Sha256};

    // Import certificate operations module
    use super::cert;
    use super::chain::{self, VerifiedChainBuilder};

    // Import OID module
    use super::cipher;
    use super::oid;

    // Import compat module (OpenSSL compatibility layer)
    use super::compat::{
        ClientConfigOptions, MultiCertResolver, ProtocolSettings, ServerConfigOptions, SslError,
        create_client_config, create_server_config, curve_name_to_kx_group, is_blocking_io_error,
        ssl_do_handshake,
    };

    use super::providers::CryptoExt;

    // Type aliases for better readability
    // Additional type alias for certificate/key pairs (SessionCache and SniCertName defined below)

    /// Certificate and private key pair used in SSL contexts
    type CertKeyPair = (Arc<CertifiedKey>, PrivateKeyDer<'static>);
    type CapathStamp = (String, Option<SystemTime>);
    type CapathCache = (Vec<CapathStamp>, Arc<Vec<Vec<u8>>>);

    #[derive(Debug, Default)]
    struct CapathState {
        directories: Vec<String>,
        cache: Option<CapathCache>,
    }

    // Constants matching Python ssl module

    // SSL/TLS Protocol versions
    #[pyattr]
    const PROTOCOL_TLS: i32 = 2; // Auto-negotiate best version
    #[pyattr]
    const PROTOCOL_SSLv23: i32 = PROTOCOL_TLS; // Alias for PROTOCOL_TLS
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

    static NEXT_SSL_SESSION_NONCE: AtomicUsize = AtomicUsize::new(1);

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
    // Use special values -2 and -1 to avoid enum name conflicts
    #[pyattr]
    const PROTO_MINIMUM_SUPPORTED: i32 = -2; // special value
    #[pyattr]
    const PROTO_MAXIMUM_SUPPORTED: i32 = -1; // special value

    // Internal constants for rustls actual supported versions
    // rustls only supports TLS 1.2 and TLS 1.3
    const MINIMUM_VERSION: i32 = PROTO_TLSv1_2; // 0x0303
    const MAXIMUM_VERSION: i32 = PROTO_TLSv1_3; // 0x0304

    // Buffer sizes and limits (OpenSSL/CPython compatibility)
    const PEM_BUFSIZE: usize = 1024;

    // OpenSSL: ssl/ssl_local.h
    const SSL3_RT_HEADER_LENGTH: usize = 5;
    // This is the maximum MAC (digest) size used by the SSL library. Currently
    // maximum of 20 is used by SHA1, but we reserve for future extension for
    // 512-bit hashes.
    const SSL3_RT_MAX_MD_SIZE: usize = 64;
    // Maximum plaintext length: defined by SSL/TLS standards
    const SSL3_RT_MAX_PLAIN_LENGTH: usize = 16384;
    // Maximum compression overhead: defined by SSL/TLS standards
    const SSL3_RT_MAX_COMPRESSED_OVERHEAD: usize = 1024;
    // The standards give a maximum encryption overhead of 1024 bytes. In
    // practice the value is lower than this. The overhead is the maximum number
    // of padding bytes (256) plus the mac size.
    const SSL3_RT_MAX_ENCRYPTED_OVERHEAD: usize = 256 + SSL3_RT_MAX_MD_SIZE;
    const SSL3_RT_MAX_COMPRESSED_LENGTH: usize =
        SSL3_RT_MAX_PLAIN_LENGTH + SSL3_RT_MAX_COMPRESSED_OVERHEAD;
    const SSL3_RT_MAX_ENCRYPTED_LENGTH: usize =
        SSL3_RT_MAX_ENCRYPTED_OVERHEAD + SSL3_RT_MAX_COMPRESSED_LENGTH;
    pub(crate) const SSL3_RT_MAX_PACKET_SIZE: usize =
        SSL3_RT_MAX_ENCRYPTED_LENGTH + SSL3_RT_HEADER_LENGTH;

    // SSL session cache size (common practice, similar to OpenSSL defaults)
    const SSL_SESSION_CACHE_SIZE: usize = 256;

    // Certificate verification modes
    #[pyattr]
    const CERT_NONE: i32 = 0;
    #[pyattr]
    const CERT_OPTIONAL: i32 = 1;
    #[pyattr]
    const CERT_REQUIRED: i32 = 2;

    // SSL Verification Flags / Certificate requirements
    #[pyattr]
    const VERIFY_DEFAULT: i32 = 0;
    #[pyattr]
    const VERIFY_CRL_CHECK_LEAF: i32 = 4;
    #[pyattr]
    const VERIFY_CRL_CHECK_CHAIN: i32 = 12;
    /// VERIFY_X509_STRICT flag for RFC 5280 strict compliance
    /// When set, performs additional validation including AKI extension checks
    #[pyattr]
    pub(crate) const VERIFY_X509_STRICT: i32 = 32;
    #[pyattr]
    const VERIFY_ALLOW_PROXY_CERTS: i32 = 64;
    #[pyattr]
    const VERIFY_X509_TRUSTED_FIRST: i32 = 32768;
    /// VERIFY_X509_PARTIAL_CHAIN flag for partial chain validation
    /// When set, accept certificates if any certificate in the chain is in the trust store
    /// (not just root CAs). This matches OpenSSL's X509_V_FLAG_PARTIAL_CHAIN behavior.
    #[pyattr]
    pub(crate) const VERIFY_X509_PARTIAL_CHAIN: i32 = 0x80000;

    // Options (OpenSSL-compatible flags, mostly no-op in rustls)
    #[pyattr]
    const OP_NO_SSLv2: i32 = 0x00000000; // Not supported anyway
    #[pyattr]
    const OP_NO_SSLv3: i32 = 0x02000000;
    #[pyattr]
    const OP_NO_TLSv1: i32 = 0x04000000;
    #[pyattr]
    const OP_NO_TLSv1_1: i32 = 0x10000000;
    #[pyattr]
    const OP_NO_TLSv1_2: i32 = 0x08000000;
    #[pyattr]
    const OP_NO_TLSv1_3: i32 = 0x20000000;
    #[pyattr]
    const OP_NO_COMPRESSION: i32 = 0x00020000;
    #[pyattr]
    const OP_CIPHER_SERVER_PREFERENCE: i32 = 0x00400000;
    #[pyattr]
    const OP_SINGLE_DH_USE: i32 = 0x00000000; // No-op in rustls
    #[pyattr]
    const OP_SINGLE_ECDH_USE: i32 = 0x00000000; // No-op in rustls
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
    const OP_ALL: i32 = 0x00000BFB; // Combined "safe" options (reduced for i32, excluding OP_LEGACY_SERVER_CONNECT for OpenSSL 3.0.0+ compatibility)

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

    // `ssl.py` still requires OpenSSL-shaped numeric compatibility fields even
    // for non-OpenSSL TLS providers. Keep them in the supported 3.x ABI range,
    // but report the actual rustls/AWS-LC backend in the human-readable string.
    #[pyattr]
    const OPENSSL_VERSION_NUMBER: i32 = 0x30300000;
    #[pyattr]
    const OPENSSL_VERSION: &str = "OpenSSL 3.3.0-compatible (AWS-LC/rustls 0.23)";
    #[pyattr]
    const OPENSSL_VERSION_INFO: (i32, i32, i32, i32, i32) = (3, 3, 0, 0, 15);
    #[pyattr]
    const _OPENSSL_API_VERSION: (i32, i32, i32, i32, i32) = (3, 3, 0, 0, 15);

    #[pyattr(once)]
    fn _DEFAULT_CIPHERS(_vm: &VirtualMachine) -> String {
        cipher::default_cipher_string()
    }

    // Has features
    #[pyattr]
    const HAS_SNI: bool = true;
    #[pyattr]
    const HAS_TLS_UNIQUE: bool = false; // Not supported
    #[pyattr]
    const HAS_ECDH: bool = true;
    #[pyattr]
    const HAS_NPN: bool = false; // Deprecated, use ALPN
    #[pyattr]
    const HAS_ALPN: bool = true;
    #[pyattr]
    const HAS_PSK: bool = false; // PSK not supported in rustls
    #[pyattr]
    const HAS_SSLv2: bool = false;
    #[pyattr]
    const HAS_SSLv3: bool = false;
    #[pyattr]
    const HAS_TLSv1: bool = false; // Not supported for security
    #[pyattr]
    const HAS_TLSv1_1: bool = false; // Not supported for security
    #[pyattr]
    const HAS_TLSv1_2: bool = true; // rustls supports TLS 1.2
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
    const ENCODING_PEM_AUX: i32 = 0x101; // PEM + 0x100

    /// Validate server hostname for TLS SNI
    ///
    /// Checks that the hostname:
    /// - Is not empty
    /// - Does not start with a dot
    /// - Is not an IP address (SNI requires DNS names)
    /// - Does not contain null bytes
    /// - Does not exceed 253 characters (DNS limit)
    ///
    /// Returns Ok(()) if validation passes, or an appropriate error.
    fn validate_hostname(hostname: &str, vm: &VirtualMachine) -> PyResult<()> {
        if hostname.is_empty() {
            return Err(vm.new_value_error("server_hostname cannot be an empty string"));
        }

        if hostname.starts_with('.') {
            return Err(vm.new_value_error("server_hostname cannot start with a dot"));
        }

        // IP addresses are allowed as server_hostname
        // SNI will not be sent for IP addresses

        if memchr(b'\0', hostname.as_bytes()).is_some() {
            cold_path();
            return Err(exceptions::nul_char_type_error(vm));
        }

        if hostname.len() > 253 {
            return Err(vm.new_value_error("server_hostname is too long (maximum 253 characters)"));
        }

        Ok(())
    }

    // SNI certificate resolver that uses shared mutable state
    // The Python SNI callback updates this state, and resolve() reads from it
    #[derive(Debug)]
    struct SniCertResolver {
        // SNI state: (certificate, server_name)
        sni_state: Arc<ParkingMutex<SniCertName>>,
    }

    impl ResolvesServerCert for SniCertResolver {
        fn resolve(&self, client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
            let mut state = self.sni_state.lock();

            // Extract and store SNI from client hello for later use
            if let Some(sni) = client_hello.server_name() {
                state.1 = Some(sni.to_string());
            } else {
                state.1 = None;
            }

            // Return the current certificate (may have been updated by Python callback)
            Some(state.0.clone())
        }
    }

    // Session data structure for tracking TLS sessions
    #[derive(Debug, Clone)]
    struct SessionData {
        _server_name: String,
        session_id: Vec<u8>,
        creation_time: SystemTime,
        lifetime: u64,
    }

    impl SessionData {
        // NOTE: This is NOT the actual TLS session ID, just a unique identifier.
        fn new(server_name: &str, lifetime: u64) -> Self {
            let creation_time = SystemTime::now();
            let nonce = NEXT_SSL_SESSION_NONCE.fetch_add(1, Ordering::Relaxed);
            let mut hasher = Sha256::new();
            hasher.update(server_name.as_bytes());
            hasher.update(
                creation_time
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
                    .to_le_bytes(),
            );
            hasher.update(nonce.to_le_bytes());

            Self {
                _server_name: server_name.to_owned(),
                session_id: hasher.finalize()[..16].to_vec(),
                creation_time,
                lifetime,
            }
        }
    }

    // Type alias to simplify complex session cache type
    type SessionCache = Arc<ParkingRwLock<HashMap<Vec<u8>, Arc<ParkingMutex<SessionData>>>>>;

    // Type alias for SNI state
    type SniCertName = (Arc<CertifiedKey>, Option<String>);

    // SESSION EMULATION IMPLEMENTATION
    //
    // IMPORTANT: This is an EMULATION of CPython's SSL session management.
    // Rustls 0.23 does NOT expose session data (ticket bytes, session IDs, etc.)
    // through public APIs. All session value fields are private.
    //
    // LIMITATIONS:
    // - Session IDs are generated from metadata (server name + timestamp hash)
    //   NOT actual TLS session IDs
    // - Ticket data is not stored (Rustls keeps it internally)
    // - Session resumption works (via Rustls's automatic mechanism)
    //   but we can't access the actual session state
    //
    // This implementation provides:
    // ✓ session.id - synthetic ID based on metadata
    // ✓ session.time - creation timestamp
    // ✓ session.timeout - default lifetime value
    // ✓ session.has_ticket - always True when session exists
    // ✓ session_reused - tracked via handshake_kind()
    // ✗ Actual TLS session ID/ticket data - NOT ACCESSIBLE

    // Custom ClientSessionStore that tracks session metadata for Python access
    // NOTE: This wraps ClientSessionMemoryCache and records metadata when sessions are stored
    #[derive(Debug)]
    struct PythonClientSessionStore {
        inner: Arc<ClientSessionMemoryCache>,
        session_cache: SessionCache,
    }

    impl PythonClientSessionStore {
        fn new(session_cache: SessionCache) -> Self {
            Self {
                inner: Arc::new(ClientSessionMemoryCache::new(SSL_SESSION_CACHE_SIZE)),
                session_cache,
            }
        }

        fn transfer_session(
            &self,
            target: &Self,
            server_name: &ServerName<'static>,
            kind: ClientSessionKind,
        ) {
            if let Some(group) = self.kx_hint(server_name) {
                target.set_kx_hint(server_name.clone(), group);
            }

            match kind {
                ClientSessionKind::Tls12 => {
                    if let Some(session) = self.tls12_session(server_name) {
                        target.set_tls12_session(server_name.clone(), session);
                    }
                }
                ClientSessionKind::Tls13 => {
                    if let Some(ticket) = self.take_tls13_ticket(server_name) {
                        target.insert_tls13_ticket(server_name.clone(), ticket);
                    }
                }
            }
        }
    }

    impl ClientSessionStore for PythonClientSessionStore {
        fn set_kx_hint(&self, server_name: ServerName<'static>, group: rustls::NamedGroup) {
            self.inner.set_kx_hint(server_name, group);
        }

        fn kx_hint(&self, server_name: &ServerName<'_>) -> Option<rustls::NamedGroup> {
            self.inner.kx_hint(server_name)
        }

        fn set_tls12_session(
            &self,
            server_name: ServerName<'static>,
            value: rustls::client::Tls12ClientSessionValue,
        ) {
            // Store in inner cache for actual resumption (Rustls handles this)
            self.inner.set_tls12_session(server_name.clone(), value);

            // Record metadata in Python-accessible cache
            // NOTE: We can't access value.session_id or value.ticket (private fields)
            // So we generate a synthetic ID from metadata
            let server_name_str = server_name.to_str();
            let session_data = SessionData::new(server_name_str.as_ref(), 7200);

            let key = server_name_str.as_bytes().to_vec();
            self.session_cache
                .write()
                .insert(key, Arc::new(ParkingMutex::new(session_data)));
        }

        fn tls12_session(
            &self,
            server_name: &ServerName<'_>,
        ) -> Option<rustls::client::Tls12ClientSessionValue> {
            self.inner.tls12_session(server_name)
        }

        fn remove_tls12_session(&self, server_name: &ServerName<'static>) {
            self.inner.remove_tls12_session(server_name);

            // Also remove from Python cache
            let key = server_name.to_str().as_bytes().to_vec();
            self.session_cache.write().remove(&key);
        }

        fn insert_tls13_ticket(
            &self,
            server_name: ServerName<'static>,
            value: rustls::client::Tls13ClientSessionValue,
        ) {
            // Store in inner cache for actual resumption (Rustls handles this)
            self.inner.insert_tls13_ticket(server_name.clone(), value);

            // Record metadata in Python-accessible cache
            // NOTE: We can't access value.ticket or value.lifetime_secs (private fields)
            // So we use default values
            let server_name_str = server_name.to_str();
            let session_data = SessionData::new(server_name_str.as_ref(), 7200);

            let key = server_name_str.as_bytes().to_vec();
            self.session_cache
                .write()
                .insert(key, Arc::new(ParkingMutex::new(session_data)));
        }

        fn take_tls13_ticket(
            &self,
            server_name: &ServerName<'static>,
        ) -> Option<rustls::client::Tls13ClientSessionValue> {
            self.inner.take_tls13_ticket(server_name)
        }
    }

    /// Parse length-prefixed ALPN protocol list
    ///
    /// Format: [len1, proto1..., len2, proto2..., ...]
    ///
    /// This is the wire format used by Python's ssl.py when calling _set_alpn_protocols().
    /// Each protocol is prefixed with a single byte indicating its length.
    ///
    /// # Arguments
    /// * `bytes` - The length-prefixed protocol data
    /// * `vm` - VirtualMachine for error creation
    ///
    /// # Returns
    /// * `Ok(Vec<Vec<u8>>)` - List of protocol names as byte vectors
    /// * `Err(PyBaseExceptionRef)` - ValueError with detailed error message
    fn parse_length_prefixed_alpn(bytes: &[u8], vm: &VirtualMachine) -> PyResult<Vec<Vec<u8>>> {
        let mut alpn_list = Vec::new();
        let mut offset = 0;

        while offset < bytes.len() {
            // Check if we can read the length byte
            if offset + 1 > bytes.len() {
                return Err(vm.new_value_error(format!(
                    "Invalid ALPN protocol data: unexpected end at offset {offset}",
                )));
            }

            let proto_len = bytes[offset] as usize;
            offset += 1;

            // Validate protocol length
            if proto_len == 0 {
                return Err(vm.new_value_error(format!(
                    "Invalid ALPN protocol data: protocol length cannot be 0 at offset {}",
                    offset - 1
                )));
            }

            // Check if we have enough bytes for the protocol data
            if offset + proto_len > bytes.len() {
                return Err(vm.new_value_error(format!(
                    "Invalid ALPN protocol data: expected {} bytes at offset {}, but only {} bytes remain",
                    proto_len, offset, bytes.len() - offset
                )));
            }

            // Extract protocol bytes
            let proto = bytes[offset..offset + proto_len].to_vec();
            alpn_list.push(proto);
            offset += proto_len;
        }

        Ok(alpn_list)
    }

    // SSLContext - manages TLS configuration
    #[pyattr]
    #[pyclass(name = "_SSLContext", module = "ssl", traverse)]
    #[derive(Debug, PyPayload)]
    struct PySSLContext {
        #[pytraverse(skip)]
        context_identity: Arc<()>,
        #[pytraverse(skip)]
        protocol: i32,
        #[pytraverse(skip)]
        check_hostname: PyRwLock<bool>,
        #[pytraverse(skip)]
        verify_mode: PyRwLock<i32>,
        #[pytraverse(skip)]
        verify_flags: PyRwLock<i32>,
        // Rustls configuration (built lazily)
        #[pytraverse(skip)]
        server_config: PyRwLock<Option<chain::ServerConfig>>,
        // Certificate store
        #[pytraverse(skip)]
        root_certs: PyRwLock<RootCertStore>,
        // Store full CA certificates for get_ca_certs()
        // RootCertStore only keeps TrustAnchors, not full certificates
        #[pytraverse(skip)]
        ca_certs_der: PyRwLock<Vec<Vec<u8>>>,
        // OpenSSL-style hashed CA directories, materialized when a connection
        // needs an immutable rustls configuration.
        #[pytraverse(skip)]
        capath_state: PyRwLock<CapathState>,
        // Certificate Revocation Lists for CRL checking
        #[pytraverse(skip)]
        crls: PyRwLock<Vec<CertificateRevocationListDer<'static>>>,
        // Server certificate/key pairs (supports multiple for RSA+ECC dual mode)
        // OpenSSL allows multiple cert/key pairs to be loaded, and selects the appropriate
        // one based on client capabilities during handshake
        // Stored as (CertifiedKey, PrivateKeyDer) to support both server and client usage
        #[pytraverse(skip)]
        cert_keys: PyRwLock<Vec<CertKeyPair>>,
        // Options
        #[pytraverse(skip)]
        options: PyRwLock<i32>,
        // ALPN protocols
        #[pytraverse(skip)]
        alpn_protocols: PyRwLock<Vec<Vec<u8>>>,
        // TLS 1.3 features
        #[pytraverse(skip)]
        post_handshake_auth: PyRwLock<bool>,
        #[pytraverse(skip)]
        num_tickets: PyRwLock<i32>,
        // Protocol version limits
        #[pytraverse(skip)]
        minimum_version: PyRwLock<i32>,
        #[pytraverse(skip)]
        maximum_version: PyRwLock<i32>,
        // SNI callback for server-side (contains PyObjectRef - needs GC tracking)
        sni_callback: PyRwLock<Option<PyObjectRef>>,
        // Message callback for debugging (contains PyObjectRef - needs GC tracking)
        msg_callback: PyRwLock<Option<PyObjectRef>>,
        // ECDH curve name for key exchange
        #[pytraverse(skip)]
        ecdh_curve: PyRwLock<Option<String>>,
        // Certificate statistics for cert_store_stats()
        #[pytraverse(skip)]
        ca_cert_count: PyRwLock<usize>, // Number of CA certificates
        #[pytraverse(skip)]
        x509_cert_count: PyRwLock<usize>, // Total number of certificates
        // Session management
        #[pytraverse(skip)]
        client_session_cache: SessionCache,
        // Rustls server session store for server-side session resumption
        #[pytraverse(skip)]
        rustls_server_session_store: Arc<rustls::server::ServerSessionMemoryCache>,
        // Shared ticketer for TLS 1.2 session tickets
        #[pytraverse(skip)]
        server_ticketer: Arc<dyn rustls::server::ProducesTickets>,
        // Server-side session statistics
        #[pytraverse(skip)]
        accept_count: AtomicUsize, // Total number of accepts
        #[pytraverse(skip)]
        session_hits: AtomicUsize, // Number of session reuses
        // Cipher suite selection
        /// Selected cipher suites (None = use all rustls defaults)
        #[pytraverse(skip)]
        selected_ciphers: PyRwLock<Option<Vec<rustls::SupportedCipherSuite>>>,
        /// Key exchange groups a SUITEB* cipher string pinned (RFC 6460)
        #[pytraverse(skip)]
        suite_b_kx_groups: PyRwLock<Option<Vec<&'static dyn SupportedKxGroup>>>,
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
    struct LoadVerifyLocationsArgs {
        #[pyarg(any, optional, error_msg = "path should be a str or bytes")]
        cafile: OptionalArg<Option<Either<PyStrRef, ArgBytesLike>>>,
        #[pyarg(any, optional, error_msg = "path should be a str or bytes")]
        capath: OptionalArg<Option<Either<PyStrRef, ArgBytesLike>>>,
        #[pyarg(any, optional, error_msg = "cadata should be a str or bytes")]
        cadata: OptionalArg<Option<Either<PyStrRef, ArgBytesLike>>>,
    }

    #[derive(FromArgs)]
    struct LoadCertChainArgs {
        #[pyarg(any, error_msg = "path should be a str or bytes")]
        certfile: Either<PyStrRef, ArgBytesLike>,
        #[pyarg(any, optional, error_msg = "path should be a str or bytes")]
        keyfile: OptionalArg<Option<Either<PyStrRef, ArgBytesLike>>>,
        #[pyarg(any, optional)]
        password: OptionalArg<PyObjectRef>,
    }

    #[derive(FromArgs)]
    struct GetCertArgs {
        #[pyarg(any, optional)]
        binary_form: OptionalArg<bool>,
    }

    #[pyclass(with(Constructor, Representable), flags(BASETYPE))]
    impl PySSLContext {
        fn warn_deprecated_tls_version(version: i32, vm: &VirtualMachine) -> PyResult<()> {
            let version_name = match version {
                PROTO_SSLv3 => Some("SSLv3"),
                PROTO_TLSv1 => Some("TLSv1"),
                PROTO_TLSv1_1 => Some("TLSv1_1"),
                _ => None,
            };
            if let Some(version_name) = version_name {
                _warnings::warn(
                    vm.ctx.exceptions.deprecation_warning,
                    format!("ssl.TLSVersion.{version_name} is deprecated"),
                    2,
                    vm,
                )?;
            }
            Ok(())
        }

        // Helper method to convert DER certificate bytes to Python dict
        fn cert_der_to_dict(&self, vm: &VirtualMachine, cert_der: &[u8]) -> PyResult<PyObjectRef> {
            cert::cert_der_to_dict_helper(vm, cert_der)
        }

        fn add_verify_dir(&self, directory: String) {
            let mut capath_state = self.capath_state.write();
            if capath_state
                .directories
                .iter()
                .any(|known| known == &directory)
            {
                return;
            }
            capath_state.directories.push(directory);
            capath_state.cache = None;
            drop(capath_state);
            *self.server_config.write() = None;
        }

        fn capath_certificates(&self) -> Arc<Vec<Vec<u8>>> {
            let directories = self.capath_state.read().directories.clone();
            if directories.is_empty() {
                return Arc::new(Vec::new());
            }

            let stamps = directories
                .iter()
                .map(|directory| {
                    let modified = rustpython_host_env::fs::metadata(directory)
                        .and_then(|metadata| metadata.modified())
                        .ok();
                    (directory.clone(), modified)
                })
                .collect::<Vec<_>>();
            if let Some((cached_stamps, certificates)) = self.capath_state.read().cache.as_ref()
                && cached_stamps == &stamps
            {
                return certificates.clone();
            }

            let mut store = RootCertStore::empty();
            let mut certificates = Vec::new();
            let mut loader = cert::CertLoader::new(&mut store, &mut certificates);
            for directory in &directories {
                let _ = loader.load_from_dir(directory);
            }
            let certificates = Arc::new(certificates);

            let mut capath_state = self.capath_state.write();
            if capath_state.directories == directories {
                capath_state.cache = Some((stamps, certificates.clone()));
            }
            certificates
        }

        fn verification_roots(&self) -> (RootCertStore, Vec<Vec<u8>>) {
            let mut root_store = self.root_certs.read().clone();
            let mut ca_certs_der = self.ca_certs_der.read().clone();
            for certificate in self.capath_certificates().iter() {
                let _ = root_store.add(certificate.clone().into());
                if !ca_certs_der.iter().any(|known| known == certificate) {
                    ca_certs_der.push(certificate.clone());
                }
            }
            (root_store, ca_certs_der)
        }

        #[pygetset]
        fn check_hostname(&self) -> bool {
            *self.check_hostname.read()
        }

        #[pygetset(setter)]
        fn set_check_hostname(&self, value: bool) {
            *self.check_hostname.write() = value;
            // When check_hostname is enabled, ensure verify_mode is at least CERT_REQUIRED
            if value {
                let current_verify_mode = *self.verify_mode.read();
                if current_verify_mode == CERT_NONE {
                    *self.verify_mode.write() = CERT_REQUIRED;
                }
            }
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
            // Cannot set CERT_NONE when check_hostname is enabled
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
            // Validate that the value is non-negative
            if value < 0 {
                return Err(vm.new_value_error("options must be non-negative"));
            }

            // Deprecated SSL/TLS protocol version options
            let opt_no = OP_NO_SSLv2
                | OP_NO_SSLv3
                | OP_NO_TLSv1
                | OP_NO_TLSv1_1
                | OP_NO_TLSv1_2
                | OP_NO_TLSv1_3;

            // Get current options and calculate newly set bits
            let old_opts = *self.options.read();
            let set = !old_opts & value; // Bits being newly set

            // Warn if any deprecated options are being newly set
            if (set & opt_no) != 0 {
                _warnings::warn(
                    vm.ctx.exceptions.deprecation_warning,
                    "ssl.OP_NO_SSL*/ssl.OP_NO_TLS* options are deprecated".to_owned(),
                    2, // stack_level = 2
                    vm,
                )?;
            }

            *self.options.write() = value;
            Ok(())
        }

        #[pygetset]
        fn minimum_version(&self) -> i32 {
            let v = *self.minimum_version.read();
            // return MINIMUM_SUPPORTED if value is 0
            if v == 0 { PROTO_MINIMUM_SUPPORTED } else { v }
        }

        #[pygetset(setter)]
        fn set_minimum_version(&self, value: i32, vm: &VirtualMachine) -> PyResult<()> {
            // Validate that the value is a valid TLS version constant
            // Valid values: 0 (default), -2 (MINIMUM_SUPPORTED), -1 (MAXIMUM_SUPPORTED),
            // or 0x0300-0x0304 (SSLv3-TLSv1.3)
            if value != 0
                && value != -2
                && value != -1
                && !(PROTO_SSLv3..=PROTO_TLSv1_3).contains(&value)
            {
                return Err(vm.new_value_error(format!("invalid protocol version: {value}")));
            }
            Self::warn_deprecated_tls_version(value, vm)?;

            // Convert special values to rustls actual supported versions
            // MINIMUM_SUPPORTED (-2) -> 0 (auto-negotiate)
            // MAXIMUM_SUPPORTED (-1) -> MAXIMUM_VERSION (TLSv1.3)
            let normalized_value = match value {
                PROTO_MINIMUM_SUPPORTED => 0,               // Auto-negotiate
                PROTO_MAXIMUM_SUPPORTED => MAXIMUM_VERSION, // TLSv1.3
                _ => value,
            };
            *self.minimum_version.write() = normalized_value;
            Ok(())
        }

        #[pygetset]
        fn maximum_version(&self) -> i32 {
            let v = *self.maximum_version.read();
            // return MAXIMUM_SUPPORTED if value is 0
            if v == 0 { PROTO_MAXIMUM_SUPPORTED } else { v }
        }

        #[pygetset(setter)]
        fn set_maximum_version(&self, value: i32, vm: &VirtualMachine) -> PyResult<()> {
            // Validate that the value is a valid TLS version constant
            // Valid values: 0 (default), -2 (MINIMUM_SUPPORTED), -1 (MAXIMUM_SUPPORTED),
            // or 0x0300-0x0304 (SSLv3-TLSv1.3)
            if value != 0
                && value != -2
                && value != -1
                && !(PROTO_SSLv3..=PROTO_TLSv1_3).contains(&value)
            {
                return Err(vm.new_value_error(format!("invalid protocol version: {value}")));
            }
            Self::warn_deprecated_tls_version(value, vm)?;

            // Convert special values to rustls actual supported versions
            // MAXIMUM_SUPPORTED (-1) -> 0 (auto-negotiate)
            // MINIMUM_SUPPORTED (-2) -> MINIMUM_VERSION (TLSv1.2)
            let normalized_value = match value {
                PROTO_MAXIMUM_SUPPORTED => 0,               // Auto-negotiate
                PROTO_MINIMUM_SUPPORTED => MINIMUM_VERSION, // TLSv1.2
                _ => value,
            };
            *self.maximum_version.write() = normalized_value;
            Ok(())
        }

        #[pymethod]
        fn load_cert_chain(&self, args: LoadCertChainArgs, vm: &VirtualMachine) -> PyResult<()> {
            let crypto_ext = CryptoExt::get_ext();

            // Parse certfile argument (str or bytes) to path
            let cert_path = Self::parse_path_arg(&args.certfile, vm)?;

            // Parse keyfile argument (default to certfile if not provided)
            let key_path = match args.keyfile {
                OptionalArg::Present(Some(ref k)) => Self::parse_path_arg(k, vm)?,
                _ => cert_path.clone(),
            };

            // Parse password argument (str, bytes-like, or callable)
            // Callable passwords are NOT invoked immediately (lazy evaluation)
            let (password_str, password_callable) =
                Self::parse_password_argument(&args.password, vm)?;

            // Validate immediate password length (limit: PEM_BUFSIZE = 1024 bytes)
            if let Some(ref pwd) = password_str
                && pwd.len() > PEM_BUFSIZE
            {
                return Err(vm.new_value_error(format!(
                    "password cannot be longer than {PEM_BUFSIZE} bytes",
                )));
            }

            // First attempt: Load with immediate password (or None if callable)
            let mut result =
                cert::load_cert_chain_from_file(&cert_path, &key_path, password_str.as_deref());

            // If failed and callable exists, invoke it and retry
            // This implements lazy evaluation: callable only invoked if password is actually needed
            if result.is_err()
                && let Some(callable) = password_callable
            {
                // Invoke callable - exceptions propagate naturally
                let pwd_result = callable.call((), vm)?;

                // Convert callable result to string
                let password_from_callable =
                    if let Ok(pwd_str) = PyUtf8StrRef::try_from_object(vm, pwd_result.clone()) {
                        pwd_str.as_str().to_owned()
                    } else if pwd_result.check_buffer() {
                        let pwd_bytes_like = ArgBytesLike::try_from_object(vm, pwd_result)?;
                        String::from_utf8(pwd_bytes_like.borrow_buf().to_vec()).map_err(|_| {
                            vm.new_type_error("password callback returned invalid UTF-8 bytes")
                        })?
                    } else {
                        return Err(
                            vm.new_type_error("password callback must return a string or bytes")
                        );
                    };

                // Validate callable password length
                if password_from_callable.len() > PEM_BUFSIZE {
                    return Err(vm.new_value_error(format!(
                        "password cannot be longer than {PEM_BUFSIZE} bytes",
                    )));
                }

                // Retry with callable password
                result = cert::load_cert_chain_from_file(
                    &cert_path,
                    &key_path,
                    Some(&password_from_callable),
                );
            }

            // Process result
            let (certs, key) = result.map_err(|e| {
                // Try to downcast to io::Error to preserve errno information
                if let Ok(io_err) = e.downcast::<std::io::Error>() {
                    match io_err.kind() {
                        // File access errors (NotFound, PermissionDenied) - preserve errno
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied => {
                            io_err.into_pyexception(vm)
                        }
                        // Other io::Error types
                        std::io::ErrorKind::Other => {
                            let msg = io_err.to_string();
                            if msg.contains("Failed to decrypt") || msg.contains("wrong password") {
                                // Wrong password error
                                vm.new_os_subtype_error(
                                    PySSLError::class(&vm.ctx).to_owned(),
                                    None,
                                    msg,
                                )
                                .upcast()
                            } else {
                                // [SSL] PEM lib
                                super::compat::SslError::create_ssl_error_with_reason(
                                    vm,
                                    Some("SSL"),
                                    "",
                                    "PEM lib",
                                )
                            }
                        }
                        // PEM parsing errors - [SSL] PEM lib
                        _ => super::compat::SslError::create_ssl_error_with_reason(
                            vm,
                            Some("SSL"),
                            "",
                            "PEM lib",
                        ),
                    }
                } else {
                    // Unknown error type - [SSL] PEM lib
                    super::compat::SslError::create_ssl_error_with_reason(
                        vm,
                        Some("SSL"),
                        "",
                        "PEM lib",
                    )
                }
            })?;

            // Validate certificate and key match
            cert::validate_cert_key_match(&certs, &key).map_err(|e| {
                let msg = if e.contains("key values mismatch") {
                    "[SSL: KEY_VALUES_MISMATCH] key values mismatch".to_owned()
                } else {
                    e
                };
                vm.new_os_subtype_error(PySSLError::class(&vm.ctx).to_owned(), Some(0), msg)
                    .upcast()
            })?;

            // Auto-build certificate chain: if only leaf cert is in file, try to add CA certs
            // This matches OpenSSL behavior where it automatically includes intermediate/CA certs
            let mut full_chain = certs.clone();
            if full_chain.len() == 1 {
                // Only have leaf cert, try to build chain from CA certs
                let ca_certs_der = self.ca_certs_der.read();
                if !ca_certs_der.is_empty() {
                    // Use build_verified_chain to construct full chain
                    let chain_result = cert::build_verified_chain(&full_chain, &ca_certs_der);
                    if chain_result.len() > 1 {
                        // Successfully built a longer chain
                        full_chain = chain_result.into_iter().map(CertificateDer::from).collect();
                    }
                }
            }

            // Additional validation: Create CertifiedKey to ensure rustls accepts it
            let signing_key = crypto_ext.any_supported_key(&key).map_err(|_| {
                vm.new_os_subtype_error(
                    PySSLError::class(&vm.ctx).to_owned(),
                    None,
                    "[SSL: KEY_VALUES_MISMATCH] key values mismatch",
                )
                .upcast()
            })?;

            let certified_key = CertifiedKey::new(full_chain.clone(), signing_key);
            if certified_key.keys_match().is_err() {
                return Err(vm
                    .new_os_subtype_error(
                        PySSLError::class(&vm.ctx).to_owned(),
                        None,
                        "[SSL: KEY_VALUES_MISMATCH] key values mismatch",
                    )
                    .upcast());
            }

            // Add cert/key pair to collection (OpenSSL allows multiple cert/key pairs)
            // Store both CertifiedKey (for server) and PrivateKeyDer (for client mTLS)
            let cert_der = &full_chain[0];
            let mut cert_keys = self.cert_keys.write();

            // Remove any existing cert/key pair with the same certificate
            // (This allows updating cert/key pair without duplicating)
            cert_keys.retain(|(existing, _)| &existing.cert[0] != cert_der);

            // Add new cert/key pair as tuple
            cert_keys.push((Arc::new(certified_key), key));

            Ok(())
        }

        #[pymethod]
        fn load_verify_locations(
            &self,
            args: LoadVerifyLocationsArgs,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            // Check that at least one argument is provided
            let has_cafile = matches!(&args.cafile, OptionalArg::Present(Some(_)));
            let has_capath = matches!(&args.capath, OptionalArg::Present(Some(_)));
            let has_cadata = matches!(&args.cadata, OptionalArg::Present(Some(_)));

            if !has_cafile && !has_capath && !has_cadata {
                return Err(vm.new_type_error("cafile, capath and cadata cannot be all omitted"));
            }

            // Parse arguments BEFORE acquiring locks to reduce lock scope
            let cafile_path = if let OptionalArg::Present(Some(ref cafile_obj)) = args.cafile {
                Some(Self::parse_path_arg(cafile_obj, vm)?)
            } else {
                None
            };

            let capath_dir = if let OptionalArg::Present(Some(ref capath_obj)) = args.capath {
                Some(Self::parse_path_arg(capath_obj, vm)?)
            } else {
                None
            };

            let cadata_parsed = if let OptionalArg::Present(Some(ref cadata_obj)) = args.cadata {
                let is_string = matches!(cadata_obj, Either::A(_));
                let data_vec = self.parse_cadata_arg(cadata_obj, vm)?;
                Some((data_vec, is_string))
            } else {
                None
            };

            // Check for CRL before acquiring main locks
            let (crl_opt, cafile_is_crl) = if let Some(ref path) = cafile_path {
                let crl = self.load_crl_from_file(path, vm)?;
                let is_crl = crl.is_some();
                (crl, is_crl)
            } else {
                (None, false)
            };

            // If it's a CRL, just add it (separate lock, no conflict with root_store)
            if let Some(crl) = crl_opt {
                self.crls.write().push(crl);
            }

            // Now acquire write locks for certificate loading
            let mut root_store = self.root_certs.write();
            let mut ca_certs_der = self.ca_certs_der.write();

            // Load from file (if not CRL)
            if let Some(ref path) = cafile_path
                && !cafile_is_crl
            {
                // Not a CRL, load as certificate
                let stats =
                    self.load_certs_from_file_helper(&mut root_store, &mut ca_certs_der, path, vm)?;
                self.update_cert_stats(stats);
            }

            // Load from bytes or str
            if let Some((ref data_vec, is_string)) = cadata_parsed {
                let stats = self.load_certs_from_bytes_helper(
                    &mut root_store,
                    &mut ca_certs_der,
                    data_vec,
                    is_string, // PEM only for strings
                    vm,
                )?;
                self.update_cert_stats(stats);
            }

            drop(root_store);
            drop(ca_certs_der);
            if let Some(dir_path) = capath_dir {
                self.add_verify_dir(dir_path);
            }
            *self.server_config.write() = None;

            Ok(())
        }

        /// Helper: Get path from Python's os.environ
        fn get_env_path(
            environ: &PyObject,
            var_name: &str,
            vm: &VirtualMachine,
        ) -> PyResult<String> {
            let path_obj = environ.get_item(var_name, vm)?;
            path_obj.try_into_value(vm)
        }

        /// Helper: Try to load certificates from Python's os.environ variables
        ///
        /// Returns true if certificates were successfully loaded from
        /// `SSL_CERT_FILE`. `SSL_CERT_DIR` is an independent source and its
        /// certificates remain hidden from store statistics until used.
        ///
        /// We use Python's os.environ instead of Rust's std::env
        /// because Python code can modify os.environ at runtime (e.g.,
        /// `os.environ['SSL_CERT_FILE'] = '/path'`), but the native certificate
        /// loader uses std::env which only sees the process environment.
        fn try_load_from_python_environ(
            &self,
            store: &mut rustls::RootCertStore,
            vm: &VirtualMachine,
        ) -> PyResult<bool> {
            let os_module = vm.import("os", 0)?;
            let environ = os_module.get_attr("environ", vm)?;

            let mut loaded_certs = Vec::new();
            let mut loaded_file = false;

            if let Ok(cert_file) = Self::get_env_path(&environ, "SSL_CERT_FILE", vm)
                && rustpython_host_env::fs::exists(&cert_file)
            {
                let mut loader = cert::CertLoader::new(store, &mut loaded_certs);
                if let Ok(stats) = loader.load_from_file(&cert_file) {
                    self.update_cert_stats(stats);
                    loaded_file = true;
                }
            }

            if let Ok(cert_dir) = Self::get_env_path(&environ, "SSL_CERT_DIR", vm)
                && rustpython_host_env::fs::is_dir(&cert_dir)
            {
                self.add_verify_dir(cert_dir);
            }

            Ok(loaded_file)
        }

        /// Helper: Load system certificates using rustls-native-certs
        ///
        /// This uses platform-specific methods:
        /// - Linux: openssl-probe to find certificate files
        /// - macOS: Keychain API
        /// - Windows: System certificate store (ROOT + CA stores)
        fn load_system_certificates(
            &self,
            store: &mut rustls::RootCertStore,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            #[cfg(windows)]
            {
                let store_names = ["ROOT", "CA"];

                for store_name in store_names {
                    let certs = rustpython_host_env::cert_store::enum_certificates(store_name);
                    for cert_ctx in certs.entries {
                        let cert = rustls::pki_types::CertificateDer::from(cert_ctx.der.to_vec());
                        let is_ca = cert::is_ca_certificate(cert.as_ref());
                        if store.add(cert).is_ok() {
                            *self.x509_cert_count.write() += 1;
                            if is_ca {
                                *self.ca_cert_count.write() += 1;
                            }
                        }
                    }
                }

                if *self.x509_cert_count.read() == 0 {
                    return Err(vm.new_os_error("Failed to load certificates from Windows store"));
                }

                Ok(())
            }

            #[cfg(not(windows))]
            {
                let result = rustpython_host_env::native_certs::load();

                // Load successfully found certificates
                for cert in result.certs {
                    let is_ca = cert::is_ca_certificate(&cert);
                    if store.add(cert.into()).is_ok() {
                        *self.x509_cert_count.write() += 1;
                        if is_ca {
                            *self.ca_cert_count.write() += 1;
                        }
                    }
                }

                // If there were errors but some certs loaded, just continue
                // If NO certs loaded and there were errors, report the first error
                if *self.x509_cert_count.read() == 0 && !result.errors.is_empty() {
                    return Err(vm.new_os_error(format!(
                        "Failed to load native certificates: {}",
                        result.errors[0]
                    )));
                }

                Ok(())
            }
        }

        #[pymethod]
        fn load_default_certs(
            &self,
            _purpose: OptionalArg<i32>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let mut store = self.root_certs.write();

            #[cfg(windows)]
            {
                // Windows: Load system certificates first, then additionally load from env
                // see: test_load_default_certs_env_windows
                let _ = self.load_system_certificates(&mut store, vm);

                let _ = self.try_load_from_python_environ(&mut store, vm)?;
            }

            #[cfg(not(windows))]
            {
                // Non-Windows: Try env vars first; only fallback to system certs if not set
                // see: test_load_default_certs_env
                let loaded = self.try_load_from_python_environ(&mut store, vm)?;

                if !loaded {
                    let _ = self.load_system_certificates(&mut store, vm);
                }
            }

            // If no certificates were loaded from system, fallback to webpki-roots (Mozilla CA bundle)
            // This ensures we always have some trusted root certificates even if system cert loading fails
            if *self.x509_cert_count.read() == 0 {
                use webpki_roots;

                // webpki_roots provides TLS_SERVER_ROOTS as &[TrustAnchor]
                // We can use extend() to add them to the RootCertStore
                let webpki_count = webpki_roots::TLS_SERVER_ROOTS.len();
                store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

                *self.x509_cert_count.write() += webpki_count;
                *self.ca_cert_count.write() += webpki_count;
            }

            drop(store);
            *self.server_config.write() = None;
            Ok(())
        }

        #[pymethod]
        fn set_alpn_protocols(&self, protocols: PyListRef, vm: &VirtualMachine) -> PyResult<()> {
            let mut alpn_list = Vec::new();
            for item in protocols.borrow_vec().iter() {
                let bytes = ArgBytesLike::try_from_object(vm, item.clone())?;
                alpn_list.push(bytes.borrow_buf().to_vec());
            }
            *self.alpn_protocols.write() = alpn_list;
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
        fn set_ciphers(&self, ciphers: PyUtf8StrRef, vm: &VirtualMachine) -> PyResult<()> {
            // `SSL_CTX_set_cipher_list` reports one failure for a string it
            // cannot read and for a readable one that selects nothing, and the
            // TLS 1.3 suites are not among what it can select -- they have
            // their own setter -- so a string naming only those selects
            // nothing either.
            let (mut selected_ciphers, suite_b_kx_groups) =
                cipher::CipherList::parse_to_rustls(ciphers.as_str())
                    .ok()
                    .filter(|(suites, _)| suites.iter().any(|s| s.tls13().is_none()))
                    .ok_or_else(|| {
                        vm.new_os_subtype_error(
                            PySSLError::class(&vm.ctx).to_owned(),
                            None,
                            "No cipher can be selected.".to_owned(),
                        )
                        .upcast()
                    })?;

            // TLS 1.3 has a separate OpenSSL setter. Discard whatever this
            // cipher string happened to match there, then restore exactly the
            // provider defaults in their preference order.
            selected_ciphers = cipher::restore_default_tls13(
                selected_ciphers,
                CryptoExt::get_ext().default_ciphers_or_provider(),
            );

            *self.selected_ciphers.write() = Some(selected_ciphers);
            *self.suite_b_kx_groups.write() = suite_b_kx_groups;
            *self.server_config.write() = None;

            Ok(())
        }

        #[pymethod]
        fn get_ciphers(&self, vm: &VirtualMachine) -> PyListRef {
            // What the last `set_ciphers` selected, or the provider defaults
            // when no cipher string was given.
            let selected = self.selected_ciphers.read().clone();
            let suites = selected
                .unwrap_or_else(|| CryptoExt::get_ext().default_ciphers_or_provider().to_vec());

            let cipher_list = suites
                .iter()
                .map(|suite| {
                    let cipher = cipher::describe(suite);
                    let dict = vm.ctx.new_dict();
                    let items: [(&str, PyObjectRef); 6] = [
                        ("id", vm.ctx.new_int(cipher.id).into()),
                        ("name", vm.ctx.new_str(cipher.name).into()),
                        ("protocol", vm.ctx.new_str(cipher.protocol).into()),
                        ("strength_bits", vm.ctx.new_int(cipher.bits).into()),
                        ("alg_bits", vm.ctx.new_int(cipher.bits).into()),
                        ("description", vm.ctx.new_str(cipher.description).into()),
                    ];
                    for (key, value) in items {
                        dict.set_item(key, value, vm).unwrap();
                    }
                    dict.into()
                })
                .collect::<Vec<_>>();

            PyListRef::from(vm.ctx.new_list(cipher_list))
        }

        #[pymethod]
        fn set_default_verify_paths(&self, vm: &VirtualMachine) -> PyResult<()> {
            // Just call load_default_certs
            self.load_default_certs(OptionalArg::Missing, vm)
        }

        #[pymethod]
        fn cert_store_stats(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            // Use the certificate counters that are updated in load_verify_locations
            let x509_count = *self.x509_cert_count.read() as i32;
            let ca_count = *self.ca_cert_count.read() as i32;

            let dict = vm.ctx.new_dict();
            dict.set_item("x509", vm.ctx.new_int(x509_count).into(), vm)?;
            dict.set_item("crl", vm.ctx.new_int(0).into(), vm)?; // CRL not supported
            dict.set_item("x509_ca", vm.ctx.new_int(ca_count).into(), vm)?;
            Ok(dict.into())
        }

        #[pymethod]
        fn session_stats(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            // Return session statistics
            // NOTE: This is a partial implementation - rustls doesn't expose all OpenSSL stats
            let dict = vm.ctx.new_dict();

            // Number of sessions currently in the cache
            let session_count = self.client_session_cache.read().len() as i32;
            dict.set_item("number", vm.ctx.new_int(session_count).into(), vm)?;

            // Client-side statistics (not tracked separately in this implementation)
            dict.set_item("connect", vm.ctx.new_int(0).into(), vm)?;
            dict.set_item("connect_good", vm.ctx.new_int(0).into(), vm)?;
            dict.set_item("connect_renegotiate", vm.ctx.new_int(0).into(), vm)?; // rustls doesn't support renegotiation

            // Server-side statistics
            let accept_count = self.accept_count.load(Ordering::SeqCst) as i32;
            dict.set_item("accept", vm.ctx.new_int(accept_count).into(), vm)?;
            dict.set_item("accept_good", vm.ctx.new_int(accept_count).into(), vm)?; // Assume all accepts are good
            dict.set_item("accept_renegotiate", vm.ctx.new_int(0).into(), vm)?; // rustls doesn't support renegotiation

            // Session reuse statistics
            let hits = self.session_hits.load(Ordering::SeqCst) as i32;
            dict.set_item("hits", vm.ctx.new_int(hits).into(), vm)?;

            // Misses, timeouts, and cache_full are not tracked in this implementation
            dict.set_item("misses", vm.ctx.new_int(0).into(), vm)?;
            dict.set_item("timeouts", vm.ctx.new_int(0).into(), vm)?;
            dict.set_item("cache_full", vm.ctx.new_int(0).into(), vm)?;

            Ok(dict.into())
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
            // Validate callback is callable or None
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
            // Alias for set_sni_callback
            self.set_sni_callback(callback, vm)
        }

        #[pygetset]
        fn security_level(&self) -> i32 {
            // rustls uses a fixed security level
            // Return 2 which is a reasonable default (equivalent to OpenSSL 1.1.0+ level 2)
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
            // Validate callback is callable or None
            if let Some(ref cb) = callback
                && !cb.is(vm.ctx.types.none_type)
                && !cb.is_callable()
            {
                return Err(vm.new_type_error("msg_callback must be callable or None"));
            }
            *self.msg_callback.write() = callback;
            Ok(())
        }

        #[pymethod]
        fn get_ca_certs(&self, args: GetCertArgs, vm: &VirtualMachine) -> PyResult<PyListRef> {
            let binary_form = args.binary_form.unwrap_or(false);
            let ca_certs_der = self.ca_certs_der.read();

            let mut certs = Vec::new();
            for cert_der in ca_certs_der.iter() {
                // Parse certificate to check if it's a CA and get info
                match x509_parser::parse_x509_certificate(cert_der) {
                    Ok((_, cert)) => {
                        // Check if this is a CA certificate (BasicConstraints: CA=TRUE)
                        let is_ca = if let Ok(Some(bc_ext)) = cert.basic_constraints() {
                            bc_ext.value.ca
                        } else {
                            false
                        };

                        // Only include CA certificates
                        if !is_ca {
                            continue;
                        }

                        if binary_form {
                            // Return DER-encoded certificate as bytes
                            certs.push(vm.ctx.new_bytes(cert_der.clone()).into());
                        } else {
                            // Return certificate as dict (use helper from _test_decode_cert)
                            let dict = self.cert_der_to_dict(vm, cert_der)?;
                            certs.push(dict);
                        }
                    }
                    Err(_) => {
                        // Skip invalid certificates
                        continue;
                    }
                }
            }

            Ok(PyListRef::from(vm.ctx.new_list(certs)))
        }

        #[pymethod]
        fn load_dh_params(&self, filepath: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            // Validate filepath is not None
            if vm.is_none(&filepath) {
                return Err(vm.new_type_error("DH params filepath cannot be None"));
            }

            // Validate filepath is str or bytes
            let path_str = if let Ok(s) = PyUtf8StrRef::try_from_object(vm, filepath.clone()) {
                s.as_str().to_owned()
            } else if filepath.check_buffer() {
                let b = ArgBytesLike::try_from_object(vm, filepath)?;
                String::from_utf8(b.borrow_buf().to_vec())
                    .map_err(|_| vm.new_value_error("Invalid path encoding"))?
            } else {
                return Err(vm.new_type_error("DH params filepath must be str or bytes"));
            };

            // Check if file exists
            if !rustpython_host_env::fs::exists(&path_str) {
                // Create FileNotFoundError with errno=ENOENT (2)
                let exc = vm.new_os_subtype_error(
                    vm.ctx.exceptions.file_not_found_error.to_owned(),
                    Some(2), // errno = ENOENT (2)
                    "No such file or directory",
                );
                // Set filename attribute
                let _ = exc
                    .as_object()
                    .set_attr("filename", vm.ctx.new_str(path_str), vm);
                return Err(exc.upcast());
            }

            // Validate that the file contains DH parameters
            // Read the file and check for DH PARAMETERS header
            let contents = rustpython_host_env::fs::read_to_string(&path_str)
                .map_err(|e| vm.new_os_error(e.to_string()))?;

            if !contents.contains("BEGIN DH PARAMETERS")
                && !contents.contains("BEGIN X9.42 DH PARAMETERS")
            {
                // File exists but doesn't contain DH parameters - raise SSLError
                // [PEM: NO_START_LINE] no start line
                return Err(super::compat::SslError::create_ssl_error_with_reason(
                    vm,
                    Some("PEM"),
                    "NO_START_LINE",
                    "[PEM: NO_START_LINE] no start line",
                ));
            }

            // rustls doesn't use DH parameters (it uses ECDHE for key exchange)
            // This is a no-op for compatibility with OpenSSL-based code
            Ok(())
        }

        #[pymethod]
        fn set_ecdh_curve(&self, name: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            // Validate name is not None
            if vm.is_none(&name) {
                return Err(vm.new_type_error("ECDH curve name cannot be None"));
            }

            // Validate name is str or bytes
            let curve_name = if let Ok(s) = PyUtf8StrRef::try_from_object(vm, name.clone()) {
                s.as_str().to_owned()
            } else if name.check_buffer() {
                let b = ArgBytesLike::try_from_object(vm, name)?;
                String::from_utf8(b.borrow_buf().to_vec())
                    .map_err(|_| vm.new_value_error("Invalid curve name encoding"))?
            } else {
                return Err(vm.new_type_error("ECDH curve name must be str or bytes"));
            };

            curve_name_to_kx_group(&curve_name).map_err(|error| vm.new_value_error(error))?;

            // Store the curve name to be used during handshake
            // This will limit the key exchange groups offered/accepted
            *self.ecdh_curve.write() = Some(curve_name);
            Ok(())
        }

        #[pymethod]
        fn _wrap_socket(
            zelf: PyRef<Self>,
            args: WrapSocketArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<PySSLSocket>> {
            let socket_mod = vm.import("socket", 0)?;
            let socket_class = socket_mod.get_attr("socket", vm)?;

            // Convert server_hostname to Option<String>
            // Handle both missing argument and None value
            let hostname = match args.server_hostname.into_option().flatten() {
                Some(hostname_str) => {
                    let hostname = hostname_str.as_str();
                    validate_hostname(hostname, vm)?;
                    Some(hostname.to_string())
                }
                None => None,
            };

            // Validate socket type and context protocol
            if args.server_side && zelf.protocol == PROTOCOL_TLS_CLIENT {
                return Err(vm
                    .new_os_subtype_error(
                        PySSLError::class(&vm.ctx).to_owned(),
                        None,
                        "Cannot create a server socket with a PROTOCOL_TLS_CLIENT context",
                    )
                    .upcast());
            }
            if !args.server_side && zelf.protocol == PROTOCOL_TLS_SERVER {
                return Err(vm
                    .new_os_subtype_error(
                        PySSLError::class(&vm.ctx).to_owned(),
                        None,
                        "Cannot create a client socket with a PROTOCOL_TLS_SERVER context",
                    )
                    .upcast());
            }

            // Create _SSLSocket instance
            let ssl_socket = PySSLSocket {
                sock: args.sock.clone(),
                sock_send_method: socket_class.get_attr("send", vm)?,
                sock_recv_method: socket_class.get_attr("recv", vm)?,
                tls_record_header_buf: vm
                    .ctx
                    .new_bytearray(Vec::with_capacity(TLS_RECORD_HEADER_SIZE))
                    .into(),
                context: PyRwLock::new(zelf),
                server_side: args.server_side,
                server_hostname: PyRwLock::new(hostname),
                connection: PyMutex::new(None),
                handshake_done: PyMutex::new(false),
                session_was_reused: PyMutex::new(false),
                owner: PyRwLock::new(
                    args.owner
                        .into_option()
                        .map(|o| o.downgrade(None, vm))
                        .transpose()?,
                ),
                session: PyRwLock::new(None),
                client_config: PyRwLock::new(None),
                chain_builder: PyRwLock::new(None),
                verified_chain: PyRwLock::new(None),
                client_session_store: PyRwLock::new(None),
                incoming_bio: None,
                outgoing_bio: None,
                sni_state: PyRwLock::new(None),
                pending_context: PyRwLock::new(None),
                client_hello_buffer: PyMutex::new(None),
                sni_callback_processed: PyMutex::new(false),
                shutdown_state: PyMutex::new(ShutdownState::NotStarted),
                pending_tls_output: PyMutex::new(Vec::new()),
                write_buffered_len: PyMutex::new(0),
                deferred_cert_error: Arc::new(ParkingRwLock::new(None)),
            };

            // Create PyRef with correct type
            let ssl_socket_ref = ssl_socket
                .into_ref_with_type(vm, vm.class("_ssl", "_SSLSocket"))
                .map_err(|_| vm.new_type_error("Failed to create SSLSocket"))?;

            if let Some(session) = args.session.into_option()
                && !vm.is_none(&session)
            {
                ssl_socket_ref.set_session(session, vm)?;
            }

            Ok(ssl_socket_ref)
        }

        #[pymethod]
        fn _wrap_bio(
            zelf: PyRef<Self>,
            args: WrapBioArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<PySSLSocket>> {
            // Convert server_hostname to Option<String>
            // Handle both missing argument and None value
            let hostname = match args.server_hostname.into_option().flatten() {
                Some(hostname_str) => {
                    let hostname = hostname_str.as_str();
                    validate_hostname(hostname, vm)?;
                    Some(hostname.to_string())
                }
                None => None,
            };

            // Extract server_side value
            let server_side = args.server_side.unwrap_or(false);

            // Validate socket type and context protocol
            if server_side && zelf.protocol == PROTOCOL_TLS_CLIENT {
                return Err(vm
                    .new_os_subtype_error(
                        PySSLError::class(&vm.ctx).to_owned(),
                        None,
                        "Cannot create a server socket with a PROTOCOL_TLS_CLIENT context",
                    )
                    .upcast());
            }
            if !server_side && zelf.protocol == PROTOCOL_TLS_SERVER {
                return Err(vm
                    .new_os_subtype_error(
                        PySSLError::class(&vm.ctx).to_owned(),
                        None,
                        "Cannot create a client socket with a PROTOCOL_TLS_SERVER context",
                    )
                    .upcast());
            }

            // Create _SSLSocket instance with BIO mode
            let ssl_socket = PySSLSocket {
                // No socket in BIO mode
                sock: vm.ctx.none(),
                sock_send_method: vm.ctx.none(),
                sock_recv_method: vm.ctx.none(),

                tls_record_header_buf: vm
                    .ctx
                    .new_bytearray(Vec::with_capacity(TLS_RECORD_HEADER_SIZE))
                    .into(),
                context: PyRwLock::new(zelf),
                server_side,
                server_hostname: PyRwLock::new(hostname),
                connection: PyMutex::new(None),
                handshake_done: PyMutex::new(false),
                session_was_reused: PyMutex::new(false),
                owner: PyRwLock::new(
                    args.owner
                        .into_option()
                        .map(|o| o.downgrade(None, vm))
                        .transpose()?,
                ),
                session: PyRwLock::new(None),
                client_config: PyRwLock::new(None),
                chain_builder: PyRwLock::new(None),
                verified_chain: PyRwLock::new(None),
                client_session_store: PyRwLock::new(None),
                incoming_bio: Some(args.incoming),
                outgoing_bio: Some(args.outgoing),
                sni_state: PyRwLock::new(None),
                pending_context: PyRwLock::new(None),
                client_hello_buffer: PyMutex::new(None),
                sni_callback_processed: PyMutex::new(false),
                shutdown_state: PyMutex::new(ShutdownState::NotStarted),
                pending_tls_output: PyMutex::new(Vec::new()),
                write_buffered_len: PyMutex::new(0),
                deferred_cert_error: Arc::new(ParkingRwLock::new(None)),
            };

            let ssl_socket_ref = ssl_socket
                .into_ref_with_type(vm, vm.class("_ssl", "_SSLSocket"))
                .map_err(|_| vm.new_type_error("Failed to create SSLSocket"))?;

            if let Some(session) = args.session.into_option()
                && !vm.is_none(&session)
            {
                ssl_socket_ref.set_session(session, vm)?;
            }

            Ok(ssl_socket_ref)
        }

        // Helper functions (private):

        /// Parse path argument (str or bytes) to string
        fn parse_path_arg(
            arg: &Either<PyStrRef, ArgBytesLike>,
            vm: &VirtualMachine,
        ) -> PyResult<String> {
            match arg {
                Either::A(s) => Ok(s.clone().try_into_utf8(vm)?.as_str().to_owned()),
                Either::B(b) => String::from_utf8(b.borrow_buf().to_vec())
                    .map_err(|_| vm.new_value_error("path contains invalid UTF-8")),
            }
        }

        /// Parse password argument (str, bytes-like, or callable)
        ///
        /// Returns (immediate_password, callable) where:
        /// - immediate_password: Some(string) if password is str/bytes, None if callable
        /// - callable: Some(PyObjectRef) if password is callable, None otherwise
        fn parse_password_argument(
            password: &OptionalArg<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<(Option<String>, Option<PyObjectRef>)> {
            match password {
                OptionalArg::Present(p) => {
                    if vm.is_none(p) {
                        return Ok((None, None));
                    }

                    // Try string
                    if let Ok(pwd_str) = PyUtf8StrRef::try_from_object(vm, p.clone()) {
                        Ok((Some(pwd_str.as_str().to_owned()), None))
                    }
                    // Try bytes-like
                    else if p.check_buffer() {
                        let pwd_bytes_like = ArgBytesLike::try_from_object(vm, p.clone())?;
                        let pwd = String::from_utf8(pwd_bytes_like.borrow_buf().to_vec())
                            .map_err(|_| vm.new_type_error("password bytes must be valid UTF-8"))?;
                        Ok((Some(pwd), None))
                    }
                    // Try callable
                    else if p.is_callable() {
                        Ok((None, Some(p.clone())))
                    } else {
                        Err(vm.new_type_error("password should be a string or callable"))
                    }
                }
                _ => Ok((None, None)),
            }
        }

        /// Helper: Load certificates from file into existing store
        fn load_certs_from_file_helper(
            &self,
            root_store: &mut RootCertStore,
            ca_certs_der: &mut Vec<Vec<u8>>,
            path: &str,
            vm: &VirtualMachine,
        ) -> PyResult<cert::CertStats> {
            let mut loader = cert::CertLoader::new(root_store, ca_certs_der);
            loader.load_from_file(path).map_err(|e| {
                // Preserve errno for file access errors (NotFound, PermissionDenied)
                match e.kind() {
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied => {
                        e.into_pyexception(vm)
                    }
                    // PEM parsing errors
                    _ => super::compat::SslError::create_ssl_error_with_reason(
                        vm,
                        Some("X509"),
                        "",
                        "PEM lib",
                    ),
                }
            })
        }

        /// Helper: Load certificates from bytes into existing store
        fn load_certs_from_bytes_helper(
            &self,
            root_store: &mut RootCertStore,
            ca_certs_der: &mut Vec<Vec<u8>>,
            data: &[u8],
            pem_only: bool,
            vm: &VirtualMachine,
        ) -> PyResult<cert::CertStats> {
            let mut loader = cert::CertLoader::new(root_store, ca_certs_der);
            // treat_all_as_ca=true: CPython counts all certificates loaded via cadata as CA certs
            // regardless of their Basic Constraints extension
            // pem_only=true for string input
            loader
                .load_from_bytes_ex(data, true, pem_only)
                .map_err(|e| {
                    // Preserve specific error messages from cert.rs
                    let err_msg = e.to_string();
                    if err_msg.contains("no start line") {
                        vm.new_os_subtype_error(
                            PySSLError::class(&vm.ctx).to_owned(),
                            None,
                            "no start line: cadata does not contain a certificate",
                        )
                        .upcast()
                    } else if err_msg.contains("not enough data") {
                        vm.new_os_subtype_error(
                            PySSLError::class(&vm.ctx).to_owned(),
                            None,
                            "not enough data: cadata does not contain a certificate",
                        )
                        .upcast()
                    } else {
                        vm.new_os_subtype_error(
                            PySSLError::class(&vm.ctx).to_owned(),
                            None,
                            err_msg,
                        )
                        .upcast()
                    }
                })
        }

        /// Helper: Try to parse data as CRL (PEM or DER format)
        fn try_parse_crl(
            &self,
            data: &[u8],
        ) -> Result<CertificateRevocationListDer<'static>, String> {
            // Try PEM format first
            let mut cursor = std::io::Cursor::new(data);
            let mut crl_iter = rustls_pemfile::crls(&mut cursor);
            if let Some(Ok(crl)) = crl_iter.next() {
                return Ok(crl);
            }

            // Try DER format
            // Basic validation: CRL should start with SEQUENCE tag (0x30)
            if !data.is_empty() && data[0] == 0x30 {
                return Ok(CertificateRevocationListDer::from(data.to_vec()));
            }

            Err("Not a valid CRL file".to_string())
        }

        /// Helper: Load CRL from file
        fn load_crl_from_file(
            &self,
            path: &str,
            vm: &VirtualMachine,
        ) -> PyResult<Option<CertificateRevocationListDer<'static>>> {
            let data = rustpython_host_env::fs::read(path).map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied => {
                    e.into_pyexception(vm)
                }
                _ => vm.new_os_error(e.to_string()),
            })?;

            match self.try_parse_crl(&data) {
                Ok(crl) => Ok(Some(crl)),
                Err(_) => Ok(None), // Not a CRL file, might be a cert file
            }
        }

        /// Helper: Parse cadata argument (str or bytes)
        fn parse_cadata_arg(
            &self,
            arg: &Either<PyStrRef, ArgBytesLike>,
            vm: &VirtualMachine,
        ) -> PyResult<Vec<u8>> {
            match arg {
                Either::A(s) => Ok(s.clone().try_into_utf8(vm)?.as_str().as_bytes().to_vec()),
                Either::B(b) => Ok(b.borrow_buf().to_vec()),
            }
        }

        /// Helper: Update certificate statistics
        fn update_cert_stats(&self, stats: cert::CertStats) {
            *self.x509_cert_count.write() += stats.total_certs;
            *self.ca_cert_count.write() += stats.ca_certs;
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
            let crypto_ext = CryptoExt::get_ext();

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

            // Set default options
            // OP_ALL | OP_NO_SSLv2 | OP_NO_SSLv3 | OP_NO_COMPRESSION |
            // OP_CIPHER_SERVER_PREFERENCE | OP_SINGLE_DH_USE | OP_SINGLE_ECDH_USE |
            // OP_ENABLE_MIDDLEBOX_COMPAT
            let default_options = OP_ALL
                | OP_NO_SSLv2
                | OP_NO_SSLv3
                | OP_NO_COMPRESSION
                | OP_CIPHER_SERVER_PREFERENCE
                | OP_SINGLE_DH_USE
                | OP_SINGLE_ECDH_USE
                | OP_ENABLE_MIDDLEBOX_COMPAT;

            // Set default verify_mode based on protocol
            // PROTOCOL_TLS_CLIENT defaults to CERT_REQUIRED
            // PROTOCOL_TLS_SERVER defaults to CERT_NONE
            let default_verify_mode = if protocol == PROTOCOL_TLS_CLIENT {
                CERT_REQUIRED
            } else {
                CERT_NONE
            };

            // Set default verify_flags based on protocol
            // Both PROTOCOL_TLS_CLIENT and PROTOCOL_TLS_SERVER only set VERIFY_X509_TRUSTED_FIRST
            // Note: VERIFY_X509_PARTIAL_CHAIN and VERIFY_X509_STRICT are NOT set here
            // - they're only added by create_default_context() in Python's ssl.py
            let default_verify_flags = VERIFY_DEFAULT | VERIFY_X509_TRUSTED_FIRST;

            // Set minimum and maximum protocol versions based on protocol constant
            // specific protocol versions fix both min and max
            let (min_version, max_version) = match protocol {
                PROTOCOL_TLSv1_2 => (PROTO_TLSv1_2, PROTO_TLSv1_2), // Only TLS 1.2
                PROTOCOL_TLSv1_3 => (PROTO_TLSv1_3, PROTO_TLSv1_3), // Only TLS 1.3
                _ => (PROTO_MINIMUM_SUPPORTED, PROTO_MAXIMUM_SUPPORTED), // Auto-negotiate
            };

            // Session metadata is scoped to the SSLContext. Each per-connection
            // rustls session store records into this shared cache.
            let shared_session_cache = Arc::new(ParkingRwLock::new(HashMap::new()));
            Ok(Self {
                context_identity: Arc::new(()),
                protocol,
                check_hostname: PyRwLock::new(protocol == PROTOCOL_TLS_CLIENT),
                verify_mode: PyRwLock::new(default_verify_mode),
                verify_flags: PyRwLock::new(default_verify_flags),
                server_config: PyRwLock::new(None),
                root_certs: PyRwLock::new(RootCertStore::empty()),
                ca_certs_der: PyRwLock::new(Vec::new()),
                capath_state: PyRwLock::new(CapathState::default()),
                crls: PyRwLock::new(Vec::new()),
                cert_keys: PyRwLock::new(Vec::new()),
                options: PyRwLock::new(default_options),
                alpn_protocols: PyRwLock::new(Vec::new()),
                post_handshake_auth: PyRwLock::new(false),
                num_tickets: PyRwLock::new(2), // TLS 1.3 default
                minimum_version: PyRwLock::new(min_version),
                maximum_version: PyRwLock::new(max_version),
                sni_callback: PyRwLock::new(None),
                msg_callback: PyRwLock::new(None),
                ecdh_curve: PyRwLock::new(None),
                ca_cert_count: PyRwLock::new(0),
                x509_cert_count: PyRwLock::new(0),
                // Use the shared cache created above
                client_session_cache: shared_session_cache,
                rustls_server_session_store: rustls::server::ServerSessionMemoryCache::new(
                    SSL_SESSION_CACHE_SIZE,
                ),
                server_ticketer: (crypto_ext.ticketer)()
                    .expect("Failed to create shared ticketer for TLS 1.2 session resumption"),
                accept_count: AtomicUsize::new(0),
                session_hits: AtomicUsize::new(0),
                selected_ciphers: PyRwLock::new(None),
                suite_b_kx_groups: PyRwLock::new(None),
            })
        }
    }

    // SSLSocket - represents a TLS-wrapped socket
    #[pyattr]
    #[pyclass(name = "_SSLSocket", module = "ssl", traverse)]
    #[derive(Debug, PyPayload)]
    pub(crate) struct PySSLSocket {
        // Underlying socket
        sock: PyObjectRef,
        // Cached socket.socket.send
        #[pytraverse(skip)]
        sock_send_method: PyObjectRef,
        // Cached socket.socket.recv
        #[pytraverse(skip)]
        sock_recv_method: PyObjectRef,
        // Header of currently read TLS record.
        #[pytraverse(skip)]
        tls_record_header_buf: PyObjectRef,
        // SSL context
        context: PyRwLock<PyRef<PySSLContext>>,
        // Server-side or client-side
        #[pytraverse(skip)]
        server_side: bool,
        // Server hostname for SNI
        #[pytraverse(skip)]
        server_hostname: PyRwLock<Option<String>>,
        // TLS connection state
        #[pytraverse(skip)]
        connection: PyMutex<Option<Connection>>,
        // Handshake completed flag
        #[pytraverse(skip)]
        handshake_done: PyMutex<bool>,
        // Session was reused (for session resumption tracking)
        #[pytraverse(skip)]
        session_was_reused: PyMutex<bool>,
        // Owner (SSLSocket instance that owns this _SSLSocket)
        owner: PyRwLock<Option<PyRef<PyWeak>>>,
        // Session for resumption
        session: PyRwLock<Option<PyObjectRef>>,
        // Client configuration used by this connection. Retained so the resulting
        // SSLSession can reuse the same verifier and client credentials.
        #[pytraverse(skip)]
        client_config: PyRwLock<Option<Arc<rustls::ClientConfig>>>,
        #[pytraverse(skip)]
        chain_builder: PyRwLock<Option<Arc<VerifiedChainBuilder>>>,
        #[pytraverse(skip)]
        verified_chain: PyRwLock<Option<Vec<Vec<u8>>>>,
        // Per-connection store containing the session selected by this connection.
        #[pytraverse(skip)]
        client_session_store: PyRwLock<Option<Arc<PythonClientSessionStore>>>,
        // MemoryBIO mode (optional)
        incoming_bio: Option<PyRef<PyMemoryBIO>>,
        outgoing_bio: Option<PyRef<PyMemoryBIO>>,
        // SNI certificate resolver state (for server-side only)
        #[pytraverse(skip)]
        sni_state: PyRwLock<Option<Arc<ParkingMutex<SniCertName>>>>,
        // Pending context change (for SNI callback deferred handling)
        pending_context: PyRwLock<Option<PyRef<PySSLContext>>>,
        // Buffer to store ClientHello for connection recreation
        #[pytraverse(skip)]
        client_hello_buffer: PyMutex<Option<Vec<u8>>>,
        // Whether the Python SNI callback has already been run for this handshake
        #[pytraverse(skip)]
        sni_callback_processed: PyMutex<bool>,
        // Shutdown state for tracking close-notify exchange
        #[pytraverse(skip)]
        shutdown_state: PyMutex<ShutdownState>,
        // Pending TLS output buffer for non-blocking sockets
        // Stores unsent TLS bytes when sock_send() would block
        // This prevents data loss when write_tls() drains rustls' internal buffer
        // but the socket cannot accept all the data immediately
        #[pytraverse(skip)]
        pub(crate) pending_tls_output: PyMutex<Vec<u8>>,
        // Tracks bytes already buffered in rustls for the current write operation
        // Prevents duplicate writes when retrying after WantWrite/WantRead
        #[pytraverse(skip)]
        pub(crate) write_buffered_len: PyMutex<usize>,
        // Deferred client certificate verification error (for TLS 1.3)
        // Stores error message if client cert verification failed during handshake
        // Error is raised on first I/O operation after handshake
        // Using Arc to share with the certificate verifier
        #[pytraverse(skip)]
        deferred_cert_error: Arc<ParkingRwLock<Option<String>>>,
    }

    // Shutdown state for tracking close-notify exchange
    #[derive(Debug, Clone, Copy, PartialEq)]
    enum ShutdownState {
        NotStarted,      // unwrap() not called yet
        SentCloseNotify, // close-notify sent, waiting for peer's response
        Completed,       // unwrap() completed successfully
    }

    /// TLS record header size (content_type + version + length).
    const TLS_RECORD_HEADER_SIZE: usize = 5;

    #[pyclass(with(Constructor, Representable), flags(BASETYPE))]
    impl PySSLSocket {
        // Check if this is BIO mode
        pub(crate) fn is_bio_mode(&self) -> bool {
            self.incoming_bio.is_some() && self.outgoing_bio.is_some()
        }

        // Get incoming BIO reference (for EOF checking)
        pub(crate) fn incoming_bio(&self) -> Option<PyObjectRef> {
            self.incoming_bio.as_ref().map(|bio| bio.clone().into())
        }

        // Check for deferred certificate verification errors (TLS 1.3)
        // If an error exists, raise it and clear it from storage
        fn check_deferred_cert_error(&self, vm: &VirtualMachine) -> PyResult<()> {
            let error_opt = self.deferred_cert_error.read().clone();
            if let Some(error_msg) = error_opt {
                // Clear the error so it's only raised once
                *self.deferred_cert_error.write() = None;
                // Raise OSError with the stored error message
                return Err(vm.new_os_error(error_msg));
            }
            Ok(())
        }

        // Get socket timeout as Duration
        pub(crate) fn get_socket_timeout(&self, vm: &VirtualMachine) -> PyResult<Option<Duration>> {
            if self.is_bio_mode() {
                return Ok(None);
            }

            // Get timeout from socket
            let timeout_obj = self.sock.get_attr("gettimeout", vm)?.call((), vm)?;

            // timeout can be None (blocking), 0.0 (non-blocking), or positive float
            if vm.is_none(&timeout_obj) {
                // None means blocking forever
                Ok(None)
            } else {
                let timeout_float: f64 = timeout_obj.try_into_value(vm)?;
                if timeout_float <= 0.0 {
                    // 0 means non-blocking
                    Ok(Some(Duration::from_secs(0)))
                } else {
                    // Positive timeout
                    Ok(Some(Duration::from_secs_f64(timeout_float)))
                }
            }
        }

        // Create and store a session object after successful handshake
        fn create_session_after_handshake(&self, was_resumed: bool, vm: &VirtualMachine) {
            // Only create session for client-side connections
            if self.server_side {
                return;
            }

            // Get server hostname
            let server_name = self.server_hostname.read().clone();
            let previous_session = self.session.read().clone();

            // Try to get session data from context's session cache
            // IMPORTANT: Acquire and release locks quickly to avoid deadlock
            let (context_identity, session_cache_arc) = {
                let context = self.context.read();
                (
                    context.context_identity.clone(),
                    context.client_session_cache.clone(),
                )
            };

            let cached_session_data = if let Some(ref name) = server_name {
                let key = name.as_bytes().to_vec();

                // Clone the data we need while holding the lock, then immediately release
                let session_data_opt = {
                    let cache_guard = session_cache_arc.read();
                    cache_guard.get(&key).cloned() // Clone Arc<PyMutex<SessionData>>
                }; // Lock released here

                if let Some(session_data_arc) = session_data_opt {
                    session_data_arc.lock().clone()
                } else {
                    SessionData::new(name, 7200)
                }
            } else {
                SessionData::new("", 7200)
            };

            let session_data = if was_resumed {
                previous_session
                    .as_ref()
                    .and_then(|session| session.downcast_ref::<PySSLSession>())
                    .map_or(cached_session_data, |session| SessionData {
                        _server_name: server_name.clone().unwrap_or_default(),
                        session_id: session.session_id.clone(),
                        creation_time: session.creation_time,
                        lifetime: session.lifetime,
                    })
            } else {
                cached_session_data
            };

            let rustls_server_name = server_name.and_then(|name| ServerName::try_from(name).ok());
            let protocol_version = self
                .connection
                .lock()
                .as_ref()
                .and_then(|connection| connection.protocol_version());
            let session_kind = match protocol_version {
                Some(rustls::ProtocolVersion::TLSv1_2) => ClientSessionKind::Tls12,
                Some(rustls::ProtocolVersion::TLSv1_3) => ClientSessionKind::Tls13,
                _ => return,
            };

            let Some(client_config) = self.client_config.write().take() else {
                return;
            };
            let Some(session_store) = self.client_session_store.write().take() else {
                return;
            };

            let session = PySSLSession {
                context_identity,
                client_config,
                chain_builder: self
                    .chain_builder
                    .read()
                    .clone()
                    .expect("connection configured"),
                session_store,
                server_name: rustls_server_name,
                kind: session_kind,
                session_id: session_data.session_id,
                creation_time: session_data.creation_time,
                lifetime: session_data.lifetime,
                has_ticket: true,
            };

            let py_session = session.into_pyobject(vm);

            *self.session.write() = Some(py_session);
        }

        /// Retain the verified path and publish its selected capath anchor.
        fn record_verified_chain(&self, was_resumed: bool) {
            let Some(builder) = self.chain_builder.read().clone() else {
                return;
            };
            // Like SSL_get0_verified_chain, resumption has no newly verified
            // path. The peer certificate remains available separately. If the
            // peer declined resumption, build the full handshake's path below.
            if was_resumed {
                *self.verified_chain.write() = None;
                return;
            }
            let chain = self.connection.lock().as_ref().and_then(|conn| {
                builder.build(
                    conn.peer_certificates()?,
                    rustls::pki_types::UnixTime::now(),
                )
            });
            *self.verified_chain.write() = chain;
            let Some(anchor) = self
                .verified_chain
                .read()
                .as_ref()
                .and_then(|chain| chain.last())
                .cloned()
            else {
                return;
            };
            let context = self.context.read();
            let mut root_certs = context.root_certs.write();
            let mut ca_certs_der = context.ca_certs_der.write();
            if !ca_certs_der.contains(&anchor) && builder.root_der.contains(&anchor) {
                let is_ca = cert::is_ca_certificate(&anchor);
                ca_certs_der.push(anchor.clone());
                let _ = root_certs.add(anchor.into());
                *context.x509_cert_count.write() += 1;
                if is_ca {
                    *context.ca_cert_count.write() += 1;
                }
            }
        }

        fn complete_handshake(&self, vm: &VirtualMachine) {
            *self.handshake_done.lock() = true;

            // Check if session was resumed - get value and release lock immediately
            let was_resumed = self
                .connection
                .lock()
                .as_ref()
                .is_some_and(|conn| conn.handshake_kind() == Some(HandshakeKind::Resumed));

            *self.session_was_reused.lock() = was_resumed;

            // Update context session statistics if server-side
            if self.server_side {
                let context = self.context.read();
                // Increment accept count for every successful server handshake
                context.accept_count.fetch_add(1, Ordering::SeqCst);
                // Increment hits count if session was resumed
                if was_resumed {
                    context.session_hits.fetch_add(1, Ordering::SeqCst);
                }
            }

            self.record_verified_chain(was_resumed);

            self.create_session_after_handshake(was_resumed, vm);
        }

        // Internal implementation with timeout control
        pub(crate) fn sock_wait_for_io_impl(
            &self,
            wait_kind: SockWaitKind,
            vm: &VirtualMachine,
        ) -> PyResult<bool> {
            if self.is_bio_mode() {
                // BIO mode doesn't use select
                return Ok(false);
            }

            // Get timeout
            let timeout = self.get_socket_timeout(vm)?;

            // Check for non-blocking mode (timeout = 0)
            if let Some(t) = timeout
                && t.is_zero()
            {
                // Non-blocking mode - don't use select
                return Ok(false);
            }

            // Use select with the effective timeout
            let py_socket: PyRef<PySocket> = self.sock.clone().try_into_value(vm)?;
            let socket = py_socket
                .sock()
                .map_err(|e| vm.new_os_error(format!("Failed to get socket: {e}")))?;

            sock_wait(&socket, wait_kind, timeout, vm)
        }

        // Internal implementation with explicit timeout override
        pub(crate) fn sock_wait_for_io_with_timeout(
            &self,
            wait_kind: SockWaitKind,
            timeout: Option<core::time::Duration>,
            vm: &VirtualMachine,
        ) -> PyResult<bool> {
            if self.is_bio_mode() {
                // BIO mode doesn't use select
                return Ok(false);
            }

            if let Some(t) = timeout
                && t.is_zero()
            {
                // Non-blocking mode - don't use select
                return Ok(false);
            }

            let py_socket: PyRef<PySocket> = self.sock.clone().try_into_value(vm)?;
            let socket = py_socket
                .sock()
                .map_err(|e| vm.new_os_error(format!("Failed to get socket: {e}")))?;

            sock_wait(&socket, wait_kind, timeout, vm).map_err(|e| e.into_pyexception(vm))
        }

        // SNI (Server Name Indication) Helper Methods:
        // These methods support the server-side handshake SNI callback mechanism

        /// Check if this is the first read during handshake (for SNI callback)
        /// Returns true until the SNI callback has been processed.
        pub(crate) fn is_first_sni_read(&self) -> bool {
            !*self.sni_callback_processed.lock()
        }

        /// Check if SNI callback is configured
        pub(crate) fn has_sni_callback(&self) -> bool {
            // Nested read locks are safe
            self.context.read().sni_callback.read().is_some()
        }

        /// Save ClientHello data for potential connection recreation.
        pub(crate) fn save_client_hello_from_bytes(&self, bytes_data: &[u8]) {
            let mut buffer = self.client_hello_buffer.lock();
            match buffer.as_mut() {
                Some(existing) => existing.extend_from_slice(bytes_data),
                None => *buffer = Some(bytes_data.to_vec()),
            }
        }

        /// Get the extracted SNI name from resolver
        pub(crate) fn get_extracted_sni_name(&self) -> Option<String> {
            // Clone the Arc option to avoid nested lock (sni_state.read -> arc.lock)
            let sni_state_opt = self.sni_state.read().clone();
            sni_state_opt.as_ref().and_then(|arc| arc.lock().1.clone())
        }

        /// Invoke the Python SNI callback
        pub(crate) fn invoke_sni_callback(
            &self,
            sni_name: Option<&str>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            // The callback may have been cleared (sni_callback = None) between the
            // handshake deciding to invoke it and this point. A concurrent removal
            // is not an error: there is simply nothing to run.
            let callback = self.context.read().sni_callback.read().clone();
            let Some(callback) = callback else {
                return Ok(());
            };

            let ssl_sock = self
                .owner
                .read()
                .as_ref()
                .and_then(|owner| owner.upgrade())
                .ok_or_else(|| {
                    super::compat::SslError::create_ssl_error_with_reason(
                        vm,
                        Some("SSL"),
                        "PARSE_TLSEXT",
                        "[SSL: PARSE_TLSEXT] SNI callback owner is no longer available",
                    )
                })?;
            let server_name_py: PyObjectRef = match sni_name {
                Some(name) => vm.ctx.new_str(name.to_string()).into(),
                None => vm.ctx.none(),
            };
            let initial_context: PyObjectRef = self.context.read().clone().into();

            // catches exceptions from the callback and reports them as unraisable
            let result = match callback.call((ssl_sock, server_name_py, initial_context), vm) {
                Ok(result) => result,
                Err(exc) => {
                    vm.run_unraisable(
                        exc,
                        Some("in ssl servername callback".to_owned()),
                        callback.clone(),
                    );
                    // Return SSL error like SSL_TLSEXT_ERR_ALERT_FATAL
                    let ssl_exc: PyBaseExceptionRef = vm
                        .new_os_subtype_error(
                            PySSLError::class(&vm.ctx).to_owned(),
                            None,
                            "SNI callback raised exception",
                        )
                        .upcast();
                    let _ = ssl_exc.as_object().set_attr(
                        "reason",
                        vm.ctx.new_str("TLSV1_ALERT_INTERNAL_ERROR"),
                        vm,
                    );
                    return Err(ssl_exc);
                }
            };

            // Check return value type (must be None or integer)
            if !vm.is_none(&result) {
                // Try to convert to integer
                if result.try_to_value::<i32>(vm).is_err() {
                    // Type conversion failed - raise TypeError as unraisable
                    let type_error = vm.new_type_error(format!(
                        "servername callback must return None or an integer, not '{}'",
                        result.class().name()
                    ));
                    vm.run_unraisable(type_error, None, result);

                    // Return SSL error with reason set to TLSV1_ALERT_INTERNAL_ERROR
                    //
                    // RUSTLS API LIMITATION:
                    // We cannot send a TLS InternalError alert to the client here because:
                    // 1. Rustls does not provide a public API like send_fatal_alert()
                    // 2. This method is called AFTER dropping the connection lock (to prevent deadlock)
                    // 3. By the time we detect the error, the connection is no longer available
                    //
                    // CPython/OpenSSL behavior:
                    // - SNI callback runs inside SSL_do_handshake with connection active
                    // - Sets *al = SSL_AD_INTERNAL_ERROR
                    // - OpenSSL automatically sends alert before returning
                    //
                    // RustPython/Rustls behavior:
                    // - SNI callback runs after dropping connection lock (deadlock prevention)
                    // - Exception has _reason='TLSV1_ALERT_INTERNAL_ERROR' for error reporting
                    // - TCP connection closes without sending TLS alert to client
                    //
                    // If rustls adds send_fatal_alert() API in the future, we should:
                    // - Re-acquire connection lock after callback
                    // - Call: connection.send_fatal_alert(AlertDescription::InternalError)
                    // - Then close connection
                    let exc: PyBaseExceptionRef = vm
                        .new_os_subtype_error(
                            PySSLError::class(&vm.ctx).to_owned(),
                            None,
                            "SNI callback returned invalid type",
                        )
                        .upcast();
                    let _ = exc.as_object().set_attr(
                        "reason",
                        vm.ctx.new_str("TLSV1_ALERT_INTERNAL_ERROR"),
                        vm,
                    );
                    return Err(exc);
                }
            }

            Ok(())
        }

        // Helper to call socket methods, bypassing any SSL wrapper
        pub(crate) fn sock_recv(&self, size: usize, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            // In BIO mode, read from incoming BIO (flags not supported)
            if let Some(ref bio) = self.incoming_bio {
                let bio_obj: PyObjectRef = bio.clone().into();
                let read_method = bio_obj.get_attr("read", vm)?;
                return read_method.call((vm.ctx.new_int(size),), vm);
            }

            self.sock_recv_method
                .call((self.sock.clone(), vm.ctx.new_int(size)), vm)
        }

        // Helper to receive data for at most one TLS record.
        // May return incomplete data but never returns more when completes a
        // previously incomplete TLS record.
        pub(crate) fn sock_recv_at_most_one_tls_record(
            &self,
            vm: &VirtualMachine,
        ) -> PyResult<PyBytesRef> {
            let obj_to_bytes = |bytes_obj| {
                PyBytesRef::try_from_object(vm, bytes_obj)
                    .map_err(|_| vm.new_os_error("Expected bytes from recv"))
            };

            let tls_record_header_buf = self
                .tls_record_header_buf
                .clone()
                .downcast::<PyByteArray>()
                .expect("BUG: tls_record_header_buf is not PyByteArray");

            let buf_len = tls_record_header_buf.borrow_buf().len();

            let (mut with_header, mut remaining_record_body_len) =
                if buf_len < TLS_RECORD_HEADER_SIZE {
                    // We do not have a full TLS record header, start receiving one.
                    let bytes_obj = self.sock_recv(TLS_RECORD_HEADER_SIZE - buf_len, vm)?;
                    let bytes = obj_to_bytes(bytes_obj)?;

                    let mut buf = tls_record_header_buf.borrow_buf_mut();
                    buf.extend_from_slice(bytes.as_bytes());

                    if buf.len() < TLS_RECORD_HEADER_SIZE {
                        return Ok(bytes);
                    }

                    // Parse the remaining length.
                    let record_body_len = u16::from_be_bytes([buf[3], buf[4]]);
                    // Validity of length value will be checked by rustls.

                    // Zero-length TLS record.
                    if record_body_len == 0 {
                        buf.clear();
                        return Ok(bytes);
                    }

                    let mut bytes_vec = bytes.as_bytes().to_vec();
                    bytes_vec.reserve(record_body_len as usize);
                    (Some(bytes_vec), record_body_len)
                } else {
                    let buf = tls_record_header_buf.borrow_buf();
                    let remaining_record_body_len = u16::from_be_bytes([buf[3], buf[4]]);
                    (None, remaining_record_body_len)
                };

            // We have full record header and are in a process of receiving a record.
            let bytes_obj = self.sock_recv(remaining_record_body_len as usize, vm)?;
            let bytes = obj_to_bytes(bytes_obj)?;

            if let Some(with_header) = with_header.as_mut() {
                with_header.extend_from_slice(bytes.as_bytes());
            }

            let mut buf = tls_record_header_buf.borrow_buf_mut();
            remaining_record_body_len -= bytes.len() as u16;
            if remaining_record_body_len == 0 {
                // Record received completely, need to start a new one beginning with its header.
                buf.clear();
            } else {
                // Update remaining length in the header.
                buf.as_mut_slice()[3..5].copy_from_slice(&remaining_record_body_len.to_be_bytes());
            }

            if let Some(with_header) = with_header {
                Ok(vm.ctx.new_bytes(with_header))
            } else {
                Ok(bytes)
            }
        }

        /// Socket send - just sends data, caller must handle pending flush
        /// Use flush_pending_tls_output before this if ordering is important
        pub(crate) fn sock_send(&self, data: &[u8], vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            // In BIO mode, write to outgoing BIO
            if let Some(ref bio) = self.outgoing_bio {
                let bio_obj: PyObjectRef = bio.clone().into();
                let write_method = bio_obj.get_attr("write", vm)?;
                return write_method.call((vm.ctx.new_bytes(data.to_vec()),), vm);
            }

            self.sock_send_method
                .call((self.sock.clone(), vm.ctx.new_bytes(data.to_vec())), vm)
        }

        /// Flush any pending TLS output data to the socket
        /// Optional deadline parameter allows respecting a read deadline during flush
        pub(crate) fn flush_pending_tls_output(
            &self,
            vm: &VirtualMachine,
            deadline: Option<std::time::Instant>,
        ) -> PyResult<()> {
            let mut pending = self.pending_tls_output.lock();
            if pending.is_empty() {
                return Ok(());
            }

            let socket_timeout = self.get_socket_timeout(vm)?;
            let is_non_blocking = socket_timeout.is_some_and(|t| t.is_zero());

            let mut sent_total = 0;

            while sent_total < pending.len() {
                // Calculate timeout: use deadline if provided, otherwise use socket timeout
                let timeout_to_use = if let Some(dl) = deadline {
                    let now = std::time::Instant::now();
                    if now >= dl {
                        // Deadline already passed
                        *pending = pending[sent_total..].to_vec();
                        return Err(
                            timeout_error_msg(vm, "The operation timed out".to_string()).upcast()
                        );
                    }
                    Some(dl - now)
                } else {
                    socket_timeout
                };

                // Use sock_wait directly with calculated timeout
                let py_socket: PyRef<PySocket> = self.sock.clone().try_into_value(vm)?;
                let socket = py_socket
                    .sock()
                    .map_err(|e| vm.new_os_error(format!("Failed to get socket: {e}")))?;
                let timed_out = sock_wait(&socket, SockWaitKind::Write, timeout_to_use, vm)?;

                if timed_out {
                    // Keep unsent data in pending buffer
                    *pending = pending[sent_total..].to_vec();
                    if is_non_blocking {
                        return Err(create_ssl_want_write_error(vm).upcast());
                    }
                    return Err(
                        timeout_error_msg(vm, "The write operation timed out".to_string()).upcast(),
                    );
                }

                match self.sock_send(&pending[sent_total..], vm) {
                    Ok(result) => {
                        let sent: usize = result.try_to_value::<isize>(vm)?.try_into().unwrap_or(0);
                        if sent == 0 {
                            if is_non_blocking {
                                // Keep unsent data in pending buffer
                                *pending = pending[sent_total..].to_vec();
                                return Err(create_ssl_want_write_error(vm).upcast());
                            }
                            // Socket said ready but sent 0 bytes - retry
                            continue;
                        }
                        sent_total += sent;
                    }
                    Err(e) => {
                        if is_blocking_io_error(&e, vm) {
                            if is_non_blocking {
                                // Keep unsent data in pending buffer
                                *pending = pending[sent_total..].to_vec();
                                return Err(create_ssl_want_write_error(vm).upcast());
                            }
                            continue;
                        }
                        // Keep unsent data in pending buffer for other errors too
                        *pending = pending[sent_total..].to_vec();
                        return Err(e);
                    }
                }
            }

            // All data sent successfully
            pending.clear();
            Ok(())
        }

        /// Send TLS output data to socket, saving unsent bytes to pending buffer
        /// This prevents data loss when rustls' write_tls() drains its internal buffer
        /// but the socket cannot accept all the data immediately
        fn send_tls_output(&self, buf: Vec<u8>, vm: &VirtualMachine) -> PyResult<()> {
            if buf.is_empty() {
                return Ok(());
            }

            let timeout = self.get_socket_timeout(vm)?;
            let is_non_blocking = timeout.is_some_and(|t| t.is_zero());

            let mut sent_total = 0;
            while sent_total < buf.len() {
                let timed_out = self.sock_wait_for_io_impl(SockWaitKind::Write, vm)?;
                if timed_out {
                    // Save unsent data to pending buffer
                    self.pending_tls_output
                        .lock()
                        .extend_from_slice(&buf[sent_total..]);
                    return Err(
                        timeout_error_msg(vm, "The write operation timed out".to_string()).upcast(),
                    );
                }

                match self.sock_send(&buf[sent_total..], vm) {
                    Ok(result) => {
                        let sent: usize = result.try_to_value::<isize>(vm)?.try_into().unwrap_or(0);
                        if sent == 0 {
                            if is_non_blocking {
                                // Save unsent data to pending buffer
                                self.pending_tls_output
                                    .lock()
                                    .extend_from_slice(&buf[sent_total..]);
                                return Err(create_ssl_want_write_error(vm).upcast());
                            }
                            continue;
                        }
                        sent_total += sent;
                    }
                    Err(e) => {
                        if is_blocking_io_error(&e, vm) {
                            if is_non_blocking {
                                // Save unsent data to pending buffer
                                self.pending_tls_output
                                    .lock()
                                    .extend_from_slice(&buf[sent_total..]);
                                return Err(create_ssl_want_write_error(vm).upcast());
                            }
                            continue;
                        }
                        // Save unsent data for other errors too
                        self.pending_tls_output
                            .lock()
                            .extend_from_slice(&buf[sent_total..]);
                        return Err(e);
                    }
                }
            }

            Ok(())
        }

        /// Flush all pending TLS output data, respecting socket timeout
        /// Used during handshake completion and shutdown() to ensure all data is sent
        pub(crate) fn blocking_flush_all_pending(&self, vm: &VirtualMachine) -> PyResult<()> {
            // Get socket timeout to respect during flush
            let timeout = self.get_socket_timeout(vm)?;
            if timeout.is_some_and(|t| t.is_zero()) {
                return self.flush_pending_tls_output(vm, None);
            }

            loop {
                let pending_data = {
                    let pending = self.pending_tls_output.lock();
                    if pending.is_empty() {
                        return Ok(());
                    }
                    pending.clone()
                };

                // Wait for socket to be writable, respecting socket timeout
                let py_socket: PyRef<PySocket> = self.sock.clone().try_into_value(vm)?;
                let socket = py_socket
                    .sock()
                    .map_err(|e| vm.new_os_error(format!("Failed to get socket: {e}")))?;
                let timed_out = sock_wait(&socket, SockWaitKind::Write, timeout, vm)?;

                if timed_out {
                    return Err(
                        timeout_error_msg(vm, "The write operation timed out".to_string()).upcast(),
                    );
                }

                // Try to send pending data (use raw to avoid recursion)
                match self.sock_send(&pending_data, vm) {
                    Ok(result) => {
                        let sent: usize = result.try_to_value::<isize>(vm)?.try_into().unwrap_or(0);
                        if sent > 0 {
                            let mut pending = self.pending_tls_output.lock();
                            pending.drain(..sent);
                        }
                        // If sent == 0, loop will retry with sock_wait
                    }
                    Err(e) => {
                        if is_blocking_io_error(&e, vm) {
                            continue;
                        }
                        return Err(e);
                    }
                }
            }
        }

        // Helper function to convert Python PROTO_* constants to rustls versions
        fn get_rustls_versions(
            minimum: i32,
            maximum: i32,
            options: i32,
        ) -> &'static [&'static rustls::SupportedProtocolVersion] {
            // Rustls only supports TLS 1.2 and 1.3
            // PROTO_TLSv1_2 = 0x0303, PROTO_TLSv1_3 = 0x0304
            // PROTO_MINIMUM_SUPPORTED = -2, PROTO_MAXIMUM_SUPPORTED = -1
            // If minimum and maximum are 0, use default (both TLS 1.2 and 1.3)

            // Static arrays for single-version configurations
            static TLS12_ONLY: &[&rustls::SupportedProtocolVersion] = &[&TLS12];
            static TLS13_ONLY: &[&rustls::SupportedProtocolVersion] = &[&TLS13];

            // Normalize special values: -2 (MINIMUM_SUPPORTED) → TLS 1.2, -1 (MAXIMUM_SUPPORTED) → TLS 1.3
            let min = if minimum == -2 {
                PROTO_TLSv1_2
            } else {
                minimum
            };
            let max = if maximum == -1 {
                PROTO_TLSv1_3
            } else {
                maximum
            };

            // Check if versions are disabled by options
            let tls12_disabled = (options & OP_NO_TLSv1_2) != 0;
            let tls13_disabled = (options & OP_NO_TLSv1_3) != 0;

            let want_tls12 = (min == 0 || min <= PROTO_TLSv1_2)
                && (max == 0 || max >= PROTO_TLSv1_2)
                && !tls12_disabled;
            let want_tls13 = (min == 0 || min <= PROTO_TLSv1_3)
                && (max == 0 || max >= PROTO_TLSv1_3)
                && !tls13_disabled;

            match (want_tls12, want_tls13) {
                (true, true) => rustls::DEFAULT_VERSIONS, // Both TLS 1.2 and 1.3
                (true, false) => TLS12_ONLY,              // Only TLS 1.2
                (false, true) => TLS13_ONLY,              // Only TLS 1.3
                (false, false) => rustls::DEFAULT_VERSIONS, // Fallback to default
            }
        }

        /// Helper: Prepare TLS versions from context settings
        fn prepare_tls_versions(&self) -> &'static [&'static rustls::SupportedProtocolVersion] {
            let ctx = self.context.read();
            let min_ver = *ctx.minimum_version.read();
            let max_ver = *ctx.maximum_version.read();
            let options = *ctx.options.read();
            Self::get_rustls_versions(min_ver, max_ver, options)
        }

        /// Helper: Prepare KX groups (ECDH curve) from context settings
        fn prepare_kx_groups(
            &self,
            vm: &VirtualMachine,
        ) -> PyResult<Option<Vec<&'static dyn SupportedKxGroup>>> {
            let ctx = self.context.read();
            let ecdh_curve = ctx.ecdh_curve.read().clone();
            let suite_b_kx_groups = ctx.suite_b_kx_groups.read().clone();
            drop(ctx);

            if let Some(ref curve_name) = ecdh_curve {
                match curve_name_to_kx_group(curve_name) {
                    Ok(groups) => Ok(Some(groups)),
                    Err(e) => Err(vm.new_value_error(format!("Failed to set ECDH curve: {e}"))),
                }
            } else {
                // A SUITEB* cipher string pins its own groups, and nothing
                // else asked for a curve.
                Ok(suite_b_kx_groups)
            }
        }

        /// Helper: Prepare all common protocol settings (versions, KX groups, ciphers, ALPN)
        fn prepare_protocol_settings(&self, vm: &VirtualMachine) -> PyResult<ProtocolSettings> {
            let ctx = self.context.read();
            let versions = self.prepare_tls_versions();
            let kx_groups = self.prepare_kx_groups(vm)?;
            let cipher_suites = ctx.selected_ciphers.read().clone();
            let alpn_protocols = ctx.alpn_protocols.read().clone();

            Ok(ProtocolSettings {
                versions,
                kx_groups,
                cipher_suites,
                alpn_protocols,
            })
        }

        /// Initialize server-side TLS connection with configuration
        ///
        /// This method handles all server-side setup including:
        /// - Certificate and key validation
        /// - Client authentication configuration
        /// - SNI (Server Name Indication) setup
        /// - ALPN protocol negotiation
        /// - Session resumption configuration
        ///
        /// Returns the configured ServerConnection.
        fn initialize_server_connection(
            &self,
            conn_guard: &mut Option<Connection>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let ctx = self.context.read();
            let cert_keys = ctx.cert_keys.read();

            if cert_keys.is_empty() {
                return Err(vm.new_value_error(
                    "Server-side connection requires certificate and key (use load_cert_chain)",
                ));
            }

            // Clone cert_keys for use in config
            // PrivateKeyDer doesn't implement Clone, use clone_key()
            let cert_keys_clone: Vec<CertKeyPair> = cert_keys
                .iter()
                .map(|(ck, pk)| (ck.clone(), pk.clone_key()))
                .collect();
            drop(cert_keys);

            // Prepare common protocol settings (TLS versions, ECDH curve, cipher suites, ALPN)
            let protocol_settings = self.prepare_protocol_settings(vm)?;
            let min_ver = *ctx.minimum_version.read();

            // Check if client certificate verification is required
            let verify_mode = *ctx.verify_mode.read();
            let (root_store, ca_certs_der) = ctx.verification_roots();
            let pha_enabled = *ctx.post_handshake_auth.read();

            // Check if TLS 1.3 is being used
            let is_tls13 = min_ver >= PROTO_TLSv1_3;

            // For TLS 1.3: always use deferred validation for client certificates
            // For TLS 1.2: use immediate validation during handshake
            let use_deferred_validation = is_tls13
                && !pha_enabled
                && (verify_mode == CERT_REQUIRED || verify_mode == CERT_OPTIONAL);

            // For TLS 1.3 + PHA: if PHA is enabled, don't request cert in initial handshake
            // The certificate will be requested later via verify_client_post_handshake()
            let request_initial_cert = if pha_enabled {
                // PHA enabled: don't request cert initially (will use PHA later)
                false
            } else if verify_mode == CERT_REQUIRED || verify_mode == CERT_OPTIONAL {
                // PHA not enabled or TLS 1.2: request cert in initial handshake
                true
            } else {
                // CERT_NONE
                false
            };

            // Check if SNI callback is set
            let sni_callback = ctx.sni_callback.read().clone();
            let use_sni_resolver = sni_callback.is_some();

            // Create SNI state if needed (to be stored in PySSLSocket later)
            // For SNI, use the first cert_key pair as the initial certificate
            let sni_state: Option<Arc<ParkingMutex<SniCertName>>> = if use_sni_resolver {
                // Use first cert_key as initial certificate for SNI
                // Extract CertifiedKey from tuple
                let (first_cert_key, _) = &cert_keys_clone[0];
                let first_cert_key = first_cert_key.clone();

                // Check if we already have existing SNI state (from previous connection)
                let existing_sni_state = self.sni_state.read().clone();

                if let Some(sni_state_arc) = existing_sni_state {
                    // Reuse existing Arc and update its contents
                    // This is crucial: rustls SniCertResolver holds references to this Arc
                    let mut state = sni_state_arc.lock();
                    state.0 = first_cert_key;
                    state.1 = None; // Reset SNI name for new connection
                    drop(state);

                    // Return the existing Arc (not a new one!)
                    Some(sni_state_arc)
                } else {
                    // First connection: create new SNI state
                    Some(Arc::new(ParkingMutex::new((first_cert_key, None))))
                }
            } else {
                None
            };

            // Determine which cert resolver to use
            // Priority: SNI > Multi-cert/Single-cert via MultiCertResolver
            let cert_resolver: Option<Arc<dyn ResolvesServerCert>> = if use_sni_resolver {
                // SNI takes precedence - use first cert_key for initial setup
                sni_state.as_ref().map(|sni_state_arc| {
                    Arc::new(SniCertResolver {
                        sni_state: sni_state_arc.clone(),
                    }) as Arc<dyn ResolvesServerCert>
                })
            } else {
                // Use MultiCertResolver for all cases (single or multiple certs)
                // Extract CertifiedKey from tuples for MultiCertResolver
                let cert_keys_only: Vec<Arc<CertifiedKey>> =
                    cert_keys_clone.iter().map(|(ck, _)| ck.clone()).collect();
                Some(Arc::new(MultiCertResolver::new(cert_keys_only)))
            };

            // Extract cert_chain and private_key from first cert_key
            //
            // Note: Since we always use cert_resolver now, these values won't actually be used
            // by create_server_config. But we still need to provide them for the API signature.
            let (first_cert_key, _) = &cert_keys_clone[0];
            let certs_clone = first_cert_key.cert.clone();

            // Provide a dummy key since cert_resolver will handle cert selection
            let key_clone = PrivateKeyDer::Pkcs8(Vec::new().into());

            // Get shared server session storage and ticketer from context
            let server_session_storage = ctx.rustls_server_session_store.clone();
            let server_ticketer = ctx.server_ticketer.clone();

            // Build server config using compat helper
            let config_options = ServerConfigOptions {
                protocol_settings,
                ca_certs_der,
                cert_chain: certs_clone,
                private_key: key_clone,
                root_store: if request_initial_cert {
                    Some(root_store)
                } else {
                    None
                },
                request_client_cert: request_initial_cert,
                use_deferred_validation,
                cert_resolver,
                deferred_cert_error: if use_deferred_validation {
                    Some(self.deferred_cert_error.clone())
                } else {
                    None
                },
                session_storage: Some(server_session_storage),
                ticketer: Some(server_ticketer),
            };

            // A capath may gain hashed entries between connections, so only
            // cache configurations whose trust store is context-owned.
            let cache_server_config =
                !use_sni_resolver && ctx.capath_state.read().directories.is_empty();
            let cached_config_arc = if cache_server_config {
                ctx.server_config.read().clone()
            } else {
                None
            };
            drop(ctx);

            let (config_arc, chain_builder) = if let Some(cached) = cached_config_arc {
                cached
            } else {
                let config =
                    create_server_config(config_options).map_err(|e| vm.new_value_error(e))?;

                if cache_server_config {
                    let ctx = self.context.read();
                    *ctx.server_config.write() = Some(config.clone());
                }

                config
            };
            *self.chain_builder.write() = Some(chain_builder);

            let conn = ServerConnection::new(config_arc).map_err(|e| {
                vm.new_value_error(format!("Failed to create server connection: {e}"))
            })?;

            *conn_guard = Some(Connection::Server(conn));

            // If ClientHello buffer exists (from SNI callback), re-inject it
            if let Some(ref hello_data) = *self.client_hello_buffer.lock()
                && let Some(Connection::Server(ref mut server)) = *conn_guard
            {
                let mut cursor = std::io::Cursor::new(hello_data.as_slice());
                let _ = server.read_tls(&mut cursor);

                // Process the re-injected ClientHello
                let _ = server.process_new_packets();

                // DON'T clear buffer - keep it to prevent callback from being invoked again
                // The buffer being non-empty signals that SNI callback was already processed
            }

            // Store SNI state if we're using SNI resolver
            if let Some(sni_state_arc) = sni_state {
                *self.sni_state.write() = Some(sni_state_arc);
            }

            Ok(())
        }

        #[pymethod]
        fn do_handshake(&self, vm: &VirtualMachine) -> PyResult<()> {
            // Check if handshake already done
            if *self.handshake_done.lock() {
                return Ok(());
            }

            let mut conn_guard = self.connection.lock();

            // Initialize connection if not already done
            if conn_guard.is_none() {
                // Check for pending context change (from SNI callback)
                let pending_context = self.pending_context.write().take();
                if let Some(new_ctx) = pending_context {
                    *self.context.write() = new_ctx;
                }

                if self.server_side {
                    // Server-side connection - delegate to helper method
                    self.initialize_server_connection(&mut conn_guard, vm)?;
                } else {
                    // Client-side connection
                    let ctx = self.context.read();

                    // Prepare common protocol settings (TLS versions, ECDH curve, cipher suites, ALPN)
                    let protocol_settings = self.prepare_protocol_settings(vm)?;

                    // Clone values we need before building config
                    let verify_mode = *ctx.verify_mode.read();
                    let (root_store_clone, ca_certs_der_clone) = ctx.verification_roots();

                    // For client mTLS: extract cert_chain and private_key from first cert_key (if any)
                    // Now we store both CertifiedKey and PrivateKeyDer as tuple
                    let cert_keys_guard = ctx.cert_keys.read();
                    let (cert_chain_clone, private_key_opt) = if !cert_keys_guard.is_empty() {
                        let (first_cert_key, private_key) = &cert_keys_guard[0];
                        let certs = first_cert_key.cert.clone();
                        (certs, Some(private_key.clone_key()))
                    } else {
                        (Vec::new(), None)
                    };
                    drop(cert_keys_guard);

                    let check_hostname = *ctx.check_hostname.read();
                    let verify_flags = *ctx.verify_flags.read();
                    let context_identity = ctx.context_identity.clone();

                    let session_cache = ctx.client_session_cache.clone();

                    // Get CRLs for revocation checking
                    let crls_clone = ctx.crls.read().clone();

                    // Drop ctx early to avoid borrow conflicts
                    drop(ctx);

                    // Parse server name for SNI
                    // Convert to ServerName
                    use rustls::pki_types::ServerName;
                    let hostname_opt = self.server_hostname.read().clone();

                    let server_name = if let Some(ref hostname) = hostname_opt {
                        // Use the provided hostname for SNI
                        ServerName::try_from(hostname.clone()).map_err(|e| {
                            vm.new_value_error(format!("Invalid server hostname: {e:?}"))
                        })?
                    } else {
                        // When server_hostname=None, use an IP address to suppress SNI
                        // no hostname = no SNI extension
                        ServerName::IpAddress(
                            core::net::IpAddr::V4(core::net::Ipv4Addr::LOCALHOST).into(),
                        )
                    };

                    let explicit_session = self.session.read().clone();
                    let session_store = Arc::new(PythonClientSessionStore::new(session_cache));
                    let (config, chain_builder) = if let Some(session) = explicit_session {
                        let session = session
                            .downcast_ref::<PySSLSession>()
                            .ok_or_else(|| vm.new_type_error("Value is not a SSLSession."))?;
                        if !Arc::ptr_eq(&session.context_identity, &context_identity) {
                            return Err(
                                vm.new_value_error("Session refers to a different SSLContext.")
                            );
                        }
                        if session.server_name.as_ref() == Some(&server_name) {
                            session.session_store.transfer_session(
                                &session_store,
                                &server_name,
                                session.kind,
                            );
                        }
                        let mut config = (*session.client_config).clone();
                        config.resumption =
                            rustls::client::Resumption::store(session_store.clone());
                        (Arc::new(config), session.chain_builder.clone())
                    } else {
                        let config_options = ClientConfigOptions {
                            protocol_settings,
                            root_store: Some(root_store_clone),
                            ca_certs_der: ca_certs_der_clone,
                            cert_chain: if !cert_chain_clone.is_empty() {
                                Some(cert_chain_clone)
                            } else {
                                None
                            },
                            private_key: private_key_opt,
                            verify_server_cert: verify_mode != CERT_NONE,
                            check_hostname,
                            verify_flags,
                            session_store: Some(session_store.clone()),
                            crls: crls_clone,
                        };
                        create_client_config(config_options).map_err(|e| vm.new_value_error(e))?
                    };

                    *self.chain_builder.write() = Some(chain_builder);
                    *self.client_config.write() = Some(config.clone());
                    *self.client_session_store.write() = Some(session_store);

                    let conn = ClientConnection::new(config, server_name).map_err(|e| {
                        vm.new_value_error(format!("Failed to create client connection: {e}"))
                    })?;

                    *conn_guard = Some(Connection::Client(conn));
                }
            }

            // Perform the actual handshake by exchanging data with the socket/BIO

            let conn = conn_guard.as_mut().expect("unreachable");
            let is_client = matches!(conn, Connection::Client(_));
            let handshake_result = ssl_do_handshake(conn, self, vm);
            drop(conn_guard);

            if is_client {
                // CLIENT is simple - no SNI callback handling needed
                handshake_result.map_err(|e| e.into_py_err(vm))?;
                self.complete_handshake(vm);
                Ok(())
            } else {
                // Use OpenSSL-compatible handshake for server
                // Handle SNI callback restart
                match handshake_result {
                    Ok(()) => {
                        // Handshake completed successfully
                        self.complete_handshake(vm);
                        Ok(())
                    }
                    Err(SslError::SniCallbackRestart) => {
                        // SNI detected - need to call callback and recreate connection

                        // Get the SNI name that was extracted (may be None if client didn't send SNI)
                        let sni_name = self.get_extracted_sni_name();

                        // Now safe to call Python callback (no locks held)
                        self.invoke_sni_callback(sni_name.as_deref(), vm)?;
                        *self.sni_callback_processed.lock() = true;

                        // Clear connection to trigger recreation
                        *self.connection.lock() = None;

                        // Recursively call do_handshake to recreate with new context
                        self.do_handshake(vm)
                    }
                    Err(e) => {
                        // Other errors - convert to Python exception
                        Err(e.into_py_err(vm))
                    }
                }
            }
        }

        #[pymethod]
        fn read(
            &self,
            len: OptionalArg<isize>,
            buffer: OptionalArg<ArgMemoryBuffer>,
            vm: &VirtualMachine,
        ) -> PyResult {
            // Convert len to usize, defaulting to 1024 if not provided
            // -1 means read all available data (treat as large buffer size)
            let len_val = len.unwrap_or(PEM_BUFSIZE as isize);
            let mut len = if len_val == -1 {
                // -1 is only valid when a buffer is provided
                match &buffer {
                    OptionalArg::Present(buf_arg) => buf_arg.len(),
                    OptionalArg::Missing => {
                        return Err(vm.new_value_error("negative read length"));
                    }
                }
            } else if len_val < 0 {
                return Err(vm.new_value_error("negative read length"));
            } else {
                len_val as usize
            };

            // if buffer is provided, limit len to buffer size
            if let OptionalArg::Present(buf_arg) = &buffer {
                let buf_len = buf_arg.len();
                if len_val <= 0 || len > buf_len {
                    len = buf_len;
                }
            }

            // return empty bytes immediately for len=0
            if len == 0 {
                return match buffer {
                    OptionalArg::Present(_) => Ok(vm.ctx.new_int(0).into()),
                    OptionalArg::Missing => Ok(vm.ctx.new_bytes(vec![]).into()),
                };
            }

            // Ensure handshake is done - if not, complete it first
            // This matches OpenSSL behavior where SSL_read() auto-completes handshake
            if !*self.handshake_done.lock() {
                self.do_handshake(vm)?;
            }

            // Check if connection has been shut down
            // Only block after shutdown is COMPLETED, not during shutdown process
            let shutdown_state = *self.shutdown_state.lock();
            if shutdown_state == ShutdownState::Completed {
                return Err(vm
                    .new_os_subtype_error(
                        PySSLError::class(&vm.ctx).to_owned(),
                        None,
                        "cannot read after shutdown",
                    )
                    .upcast());
            }

            // Helper function to handle return value based on buffer presence
            let return_data = |data: Vec<u8>,
                               buffer_arg: &OptionalArg<ArgMemoryBuffer>,
                               vm: &VirtualMachine|
             -> PyResult<PyObjectRef> {
                match buffer_arg {
                    OptionalArg::Present(buf_arg) => {
                        // Write into buffer and return number of bytes written
                        let n = data.len();
                        if n > 0 {
                            let mut buf = buf_arg.borrow_buf_mut();
                            let buf_slice = &mut *buf;
                            let copy_len = n.min(buf_slice.len());
                            buf_slice[..copy_len].copy_from_slice(&data[..copy_len]);
                        }
                        Ok(vm.ctx.new_int(n).into())
                    }
                    OptionalArg::Missing => {
                        // Return bytes object
                        Ok(vm.ctx.new_bytes(data).into())
                    }
                }
            };

            // Use compat layer for unified read logic with proper EOF handling
            // This matches SSL_read_ex() approach
            let mut buf = vm.new_zeroed_bytes(len)?;
            let read_result = {
                let mut conn_guard = self.connection.lock();
                let conn = conn_guard
                    .as_mut()
                    .ok_or_else(|| vm.new_value_error("Connection not established"))?;
                crate::ssl::compat::ssl_read(conn, &mut buf, self, vm)
            };
            match read_result {
                Ok(n) => {
                    // Check for deferred certificate verification errors (TLS 1.3)
                    // Must be checked AFTER ssl_read, as the error is set during I/O
                    self.check_deferred_cert_error(vm)?;
                    buf.truncate(n);
                    return_data(buf, &buffer, vm)
                }
                Err(crate::ssl::compat::SslError::Eof) => {
                    // If plaintext is still buffered, return it before EOF.
                    let pending = {
                        let mut conn_guard = self.connection.lock();
                        let conn = match conn_guard.as_mut() {
                            Some(conn) => conn,
                            None => return Err(create_ssl_eof_error(vm).upcast()),
                        };

                        let mut reader = conn.reader();
                        reader.fill_buf().map_or(0, |buf| buf.len())
                    };
                    if pending > 0 {
                        let mut buf = vec![0u8; pending.min(len)];
                        let read_retry = {
                            let mut conn_guard = self.connection.lock();
                            let conn = conn_guard
                                .as_mut()
                                .ok_or_else(|| vm.new_value_error("Connection not established"))?;
                            crate::ssl::compat::ssl_read(conn, &mut buf, self, vm)
                        };
                        if let Ok(n) = read_retry {
                            buf.truncate(n);
                            return return_data(buf, &buffer, vm);
                        }
                    }
                    // EOF occurred in violation of protocol (unexpected closure)
                    Err(create_ssl_eof_error(vm).upcast())
                }
                Err(crate::ssl::compat::SslError::ZeroReturn) => {
                    // If plaintext is still buffered, return it before clean EOF.
                    let pending = {
                        let mut conn_guard = self.connection.lock();
                        let conn = match conn_guard.as_mut() {
                            Some(conn) => conn,
                            None => return Err(create_ssl_zero_return_error(vm).upcast()),
                        };

                        let mut reader = conn.reader();
                        reader.fill_buf().map_or(0, |buf| buf.len())
                    };
                    if pending > 0 {
                        let mut buf = vec![0u8; pending.min(len)];
                        let read_retry = {
                            let mut conn_guard = self.connection.lock();
                            let conn = conn_guard
                                .as_mut()
                                .ok_or_else(|| vm.new_value_error("Connection not established"))?;
                            crate::ssl::compat::ssl_read(conn, &mut buf, self, vm)
                        };
                        if let Ok(n) = read_retry {
                            buf.truncate(n);
                            return return_data(buf, &buffer, vm);
                        }
                    }
                    // Clean closure via close_notify from peer.
                    // If we already sent close_notify (unwrap was called),
                    // raise SSLZeroReturnError (bidirectional shutdown).
                    // Otherwise return empty bytes, which callers (asyncore,
                    // asyncio sslproto) interpret as EOF.
                    let our_shutdown_state = *self.shutdown_state.lock();
                    if our_shutdown_state == ShutdownState::SentCloseNotify
                        || our_shutdown_state == ShutdownState::Completed
                    {
                        Err(create_ssl_zero_return_error(vm).upcast())
                    } else {
                        return_data(vec![], &buffer, vm)
                    }
                }
                Err(crate::ssl::compat::SslError::WantRead) => {
                    // Non-blocking mode: would block
                    Err(create_ssl_want_read_error(vm).upcast())
                }
                Err(crate::ssl::compat::SslError::WantWrite) => {
                    // Non-blocking mode: would block on write
                    Err(create_ssl_want_write_error(vm).upcast())
                }
                Err(crate::ssl::compat::SslError::Timeout(msg)) => {
                    Err(timeout_error_msg(vm, msg).upcast())
                }
                Err(crate::ssl::compat::SslError::Py(e)) => {
                    // Python exception - pass through
                    Err(e)
                }
                Err(e) => {
                    // Other SSL errors
                    Err(e.into_py_err(vm))
                }
            }
        }

        #[pymethod]
        fn pending(&self) -> usize {
            // Returns the number of already decrypted bytes available for read
            // This is critical for asyncore's readable() method which checks socket.pending() > 0
            let mut conn_guard = self.connection.lock();
            let conn = match conn_guard.as_mut() {
                Some(c) => c,
                None => return 0, // No connection established yet
            };

            // Use rustls Reader's fill_buf() to check buffered plaintext
            // fill_buf() returns a reference to buffered data without consuming it
            // This matches OpenSSL's SSL_pending() behavior
            let mut reader = conn.reader();
            match reader.fill_buf() {
                Ok(buf) => buf.len(),
                Err(_) => {
                    // WouldBlock or other errors mean no data available
                    // Return 0 like OpenSSL does when buffer is empty
                    0
                }
            }
        }

        #[pymethod]
        fn write(&self, data: ArgBytesLike, vm: &VirtualMachine) -> PyResult<usize> {
            let data_bytes = data.borrow_buf();
            let data_len = data_bytes.len();

            if data_len == 0 {
                return Ok(0);
            }

            // Ensure handshake is done (SSL_write auto-completes handshake)
            if !*self.handshake_done.lock() {
                self.do_handshake(vm)?;
            }

            // Check shutdown state
            // Only block after shutdown is COMPLETED, not during shutdown process
            if *self.shutdown_state.lock() == ShutdownState::Completed {
                return Err(vm
                    .new_os_subtype_error(
                        PySSLError::class(&vm.ctx).to_owned(),
                        None,
                        "cannot write after shutdown",
                    )
                    .upcast());
            }

            // Call ssl_write (matches CPython's SSL_write_ex loop)
            let result = {
                let mut conn_guard = self.connection.lock();
                let conn = conn_guard
                    .as_mut()
                    .ok_or_else(|| vm.new_value_error("Connection not established"))?;

                crate::ssl::compat::ssl_write(conn, data_bytes.as_ref(), self, vm)
            };

            match result {
                Ok(n) => {
                    self.check_deferred_cert_error(vm)?;
                    Ok(n)
                }
                Err(crate::ssl::compat::SslError::WantRead) => {
                    Err(create_ssl_want_read_error(vm).upcast())
                }
                Err(crate::ssl::compat::SslError::WantWrite) => {
                    Err(create_ssl_want_write_error(vm).upcast())
                }
                Err(crate::ssl::compat::SslError::Timeout(msg)) => {
                    Err(timeout_error_msg(vm, msg).upcast())
                }
                Err(e) => Err(e.into_py_err(vm)),
            }
        }

        #[pymethod]
        fn getpeercert(
            &self,
            args: GetCertArgs,
            vm: &VirtualMachine,
        ) -> PyResult<Option<PyObjectRef>> {
            let binary = args.binary_form.unwrap_or(false);

            // Check if handshake is complete
            if !*self.handshake_done.lock() {
                return Err(vm.new_value_error("handshake not done yet"));
            }

            // Extract DER bytes from connection, releasing lock quickly
            let der_bytes = {
                let conn_guard = self.connection.lock();
                let conn = conn_guard
                    .as_ref()
                    .ok_or_else(|| vm.new_value_error("No TLS connection established"))?;

                let Some(peer_certificates) = conn.peer_certificates() else {
                    return Ok(None);
                };
                let cert = peer_certificates
                    .first()
                    .ok_or_else(|| vm.new_value_error("No peer certificate available"))?;
                cert.as_ref().to_vec()
            };

            if binary {
                // Return DER-encoded certificate as bytes
                return Ok(Some(vm.ctx.new_bytes(der_bytes).into()));
            }

            // Dictionary mode: check verify_mode
            let verify_mode = *self.context.read().verify_mode.read();

            if verify_mode == CERT_NONE {
                // Return empty dict when CERT_NONE
                return Ok(Some(vm.ctx.new_dict().into()));
            }

            // Parse DER certificate and convert to dict (outside lock)
            let (_, cert) = x509_parser::parse_x509_certificate(&der_bytes)
                .map_err(|e| vm.new_value_error(format!("Failed to parse certificate: {e}")))?;

            cert::cert_to_dict(vm, &cert).map(Some)
        }

        #[pymethod]
        fn cipher(&self) -> Option<(String, String, i32)> {
            // Extract cipher suite, releasing lock quickly
            let suite = {
                let conn_guard = self.connection.lock();
                conn_guard.as_ref()?.negotiated_cipher_suite()?
            };

            // Extract cipher information outside the lock
            let cipher = cipher::describe(&suite);

            // Note: returns a 3-tuple (name, protocol_version, bits)
            // The 'description' field is part of get_ciphers() output, not cipher()
            Some((
                cipher.name.to_owned(),
                cipher.protocol.to_owned(),
                cipher.bits.into(),
            ))
        }

        #[pymethod]
        fn version(&self) -> Option<String> {
            // Extract cipher suite, releasing lock quickly
            let suite = {
                let conn_guard = self.connection.lock();
                conn_guard.as_ref()?.negotiated_cipher_suite()?
            };

            // Convert to string outside the lock
            let version_str = match suite.version().version {
                rustls::ProtocolVersion::TLSv1_2 => "TLSv1.2",
                rustls::ProtocolVersion::TLSv1_3 => "TLSv1.3",
                _ => "Unknown",
            };

            Some(version_str.to_string())
        }

        #[pymethod]
        fn selected_alpn_protocol(&self) -> Option<String> {
            let conn_guard = self.connection.lock();
            let conn = conn_guard.as_ref()?;

            let alpn_bytes = conn.alpn_protocol()?;

            // Null byte protocol (vec![0u8]) means no actual ALPN match (fallback protocol)
            if alpn_bytes.is_empty() || alpn_bytes == [0u8] {
                return None;
            }

            // Convert bytes to string
            String::from_utf8(alpn_bytes.to_vec()).ok()
        }

        #[pymethod]
        fn selected_npn_protocol(&self) -> Option<String> {
            // NPN (Next Protocol Negotiation) is the predecessor to ALPN
            // It was deprecated in favor of ALPN (RFC 7301)
            // Rustls doesn't support NPN, only ALPN
            // Return None to indicate NPN is not supported
            None
        }

        #[pygetset]
        fn owner(&self) -> Option<PyObjectRef> {
            self.owner.read().as_ref().and_then(|owner| owner.upgrade())
        }

        #[pygetset(setter)]
        fn set_owner(&self, owner: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            *self.owner.write() = Some(owner.downgrade(None, vm)?);
            Ok(())
        }

        #[pygetset]
        fn server_side(&self) -> bool {
            self.server_side
        }

        #[pygetset]
        fn context(&self) -> PyRef<PySSLContext> {
            self.context.read().clone()
        }

        #[pygetset(setter)]
        fn set_context(&self, value: PyRef<PySSLContext>, _vm: &VirtualMachine) {
            // Update context reference immediately
            // SSL_set_SSL_CTX allows context changes at any time,
            // even after handshake completion
            *self.context.write() = value;

            // Clear pending context as we've applied the change
            *self.pending_context.write() = None;
        }

        #[pygetset]
        fn server_hostname(&self) -> Option<String> {
            self.server_hostname.read().clone()
        }

        #[pygetset(setter)]
        fn set_server_hostname(
            &self,
            value: Option<PyUtf8StrRef>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            // Check if handshake is already done
            if *self.handshake_done.lock() {
                return Err(
                    vm.new_value_error("Cannot set server_hostname on socket after handshake")
                );
            }

            // Validate hostname
            let hostname_string = value
                .map(|s| {
                    validate_hostname(s.as_str(), vm)?;
                    Ok::<String, _>(s.as_str().to_owned())
                })
                .transpose()?;

            *self.server_hostname.write() = hostname_string;
            Ok(())
        }

        #[pygetset]
        fn session(&self, vm: &VirtualMachine) -> PyObjectRef {
            // Return the stored session object if any
            let sess = self.session.read().clone();
            sess.unwrap_or_else(|| vm.ctx.none())
        }

        #[pygetset(setter)]
        fn set_session(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            let session = value
                .downcast_ref::<PySSLSession>()
                .ok_or_else(|| vm.new_type_error("Value is not a SSLSession."))?;

            if !Arc::ptr_eq(
                &session.context_identity,
                &self.context.read().context_identity,
            ) {
                return Err(vm.new_value_error("Session refers to a different SSLContext."));
            }

            // Check if this is a client socket
            if self.server_side {
                return Err(vm.new_value_error("Cannot set session for server-side SSLSocket"));
            }

            // Check if handshake is already done
            if *self.handshake_done.lock() {
                return Err(vm.new_value_error("Cannot set session after handshake."));
            }

            // Store the session for potential use during handshake
            *self.session.write() = Some(value);

            Ok(())
        }

        #[pygetset]
        fn session_reused(&self) -> bool {
            // Return the tracked session reuse status
            *self.session_was_reused.lock()
        }

        #[pymethod]
        fn compression(&self) -> Option<&'static str> {
            // rustls doesn't support compression
            None
        }

        #[pymethod]
        fn get_unverified_chain(&self, vm: &VirtualMachine) -> PyResult<Option<PyListRef>> {
            // Get peer certificates from the connection
            let conn_guard = self.connection.lock();
            let conn = conn_guard
                .as_ref()
                .ok_or_else(|| vm.new_value_error("Handshake not completed"))?;

            let certs = conn.peer_certificates();

            let Some(certs) = certs else {
                return Ok(None);
            };

            // Convert to list of Certificate objects
            let cert_list: Vec<PyObjectRef> = certs
                .iter()
                .map(|cert_der| {
                    let cert_bytes = cert_der.as_ref().to_vec();
                    PySSLCertificate {
                        der_bytes: cert_bytes,
                    }
                    .into_ref(&vm.ctx)
                    .into()
                })
                .collect();

            Ok(Some(vm.ctx.new_list(cert_list)))
        }

        #[pymethod]
        fn get_verified_chain(&self, vm: &VirtualMachine) -> Option<PyListRef> {
            let chain_der = self.verified_chain.read().clone()?;

            // Convert DER chain to Python list of Certificate objects
            let cert_list: Vec<PyObjectRef> = chain_der
                .into_iter()
                .map(|der_bytes| PySSLCertificate { der_bytes }.into_ref(&vm.ctx).into())
                .collect();

            Some(vm.ctx.new_list(cert_list))
        }

        #[pymethod]
        fn shutdown(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            // Check current shutdown state
            let current_state = *self.shutdown_state.lock();

            // If already completed, return immediately
            if current_state == ShutdownState::Completed {
                if self.is_bio_mode() {
                    return Ok(vm.ctx.none());
                }
                return Ok(self.sock.clone());
            }

            // Get connection
            let mut conn_guard = self.connection.lock();
            let conn = conn_guard
                .as_mut()
                .ok_or_else(|| vm.new_value_error("Connection not established"))?;

            let is_bio = self.is_bio_mode();

            // Step 1: Send our close_notify if not already sent
            if current_state == ShutdownState::NotStarted {
                // First, flush ALL pending TLS data BEFORE sending close_notify
                // This is CRITICAL - close_notify must come AFTER all application data
                // Otherwise data loss occurs when peer receives close_notify first

                // Step 1a: Flush any pending TLS records from rustls internal buffer
                // This ensures all application data is converted to TLS records
                while conn.wants_write() {
                    let mut buf = Vec::new();
                    conn.write_tls(&mut buf)
                        .map_err(|e| vm.new_os_error(format!("TLS write failed: {e}")))?;
                    if !buf.is_empty() {
                        self.send_tls_output(buf, vm)?;
                    }
                }

                // Step 1b: Flush pending_tls_output buffer to socket
                if !is_bio {
                    // Socket mode: blocking flush to ensure data order
                    // Must complete before sending close_notify
                    self.blocking_flush_all_pending(vm)?;
                } else {
                    // BIO mode: non-blocking flush (caller handles pending data)
                    let _ = self.flush_pending_tls_output(vm, None);
                }

                conn.send_close_notify();

                // Write close_notify to outgoing buffer/BIO
                self.write_pending_tls(conn, vm)?;
                // Ensure close_notify and any pending TLS data are flushed
                if !is_bio {
                    self.flush_pending_tls_output(vm, None)?;
                }

                // Update state
                *self.shutdown_state.lock() = ShutdownState::SentCloseNotify;
            }

            // Step 2: Try to read and process peer's close_notify

            // First check if we already have peer's close_notify
            // This can happen if it was received during a previous read() call
            let mut peer_closed = self.check_peer_closed(conn, vm)?;

            // If peer hasn't closed yet, try to read from socket
            if !peer_closed {
                // Check socket timeout mode
                let timeout_mode = if !is_bio {
                    // Get socket timeout
                    match self.sock.get_attr("gettimeout", vm) {
                        Ok(method) => match method.call((), vm) {
                            Ok(timeout) => {
                                if vm.is_none(&timeout) {
                                    // timeout=None means blocking
                                    Some(None)
                                } else if let Ok(t) = timeout.try_float(vm).map(|f| f.to_f64()) {
                                    if t == 0.0 {
                                        // timeout=0 means non-blocking
                                        Some(Some(0.0))
                                    } else {
                                        // timeout>0 means timeout mode
                                        Some(Some(t))
                                    }
                                } else {
                                    None
                                }
                            }
                            Err(_) => None,
                        },
                        Err(_) => None,
                    }
                } else {
                    None // BIO mode
                };

                if is_bio {
                    // In BIO mode: non-blocking read attempt
                    if self.try_read_close_notify(conn, vm)? {
                        peer_closed = true;
                    }
                } else if let Some(timeout) = timeout_mode {
                    match timeout {
                        Some(0.0) => {
                            // Non-blocking: return immediately after sending close_notify.
                            // Don't wait for peer's close_notify to avoid blocking.
                            drop(conn_guard);
                            // Best-effort flush; WouldBlock is expected in non-blocking mode.
                            // Other errors indicate close_notify may not have been sent,
                            // but we still complete shutdown to avoid inconsistent state.
                            let _ = self.flush_pending_tls_output(vm, None);
                            *self.shutdown_state.lock() = ShutdownState::Completed;
                            *self.connection.lock() = None;
                            return Ok(self.sock.clone());
                        }
                        _ => {
                            // Blocking or timeout mode: wait for peer's close_notify.
                            // This is proper TLS shutdown - we should receive peer's
                            // close_notify before closing the connection.
                            drop(conn_guard);

                            // Flush our close_notify first
                            if timeout.is_none() {
                                self.blocking_flush_all_pending(vm)?;
                            } else {
                                self.flush_pending_tls_output(vm, None)?;
                            }

                            // Calculate deadline for timeout mode
                            let deadline = timeout.map(|t| {
                                std::time::Instant::now() + core::time::Duration::from_secs_f64(t)
                            });

                            // Wait for peer's close_notify
                            loop {
                                // Re-acquire connection lock for each iteration
                                let mut conn_guard = self.connection.lock();
                                let conn = match conn_guard.as_mut() {
                                    Some(c) => c,
                                    None => break, // Connection already closed
                                };

                                // Check if peer already sent close_notify
                                if self.check_peer_closed(conn, vm)? {
                                    break;
                                }

                                drop(conn_guard);

                                // Check timeout
                                let remaining_timeout = if let Some(dl) = deadline {
                                    let now = std::time::Instant::now();
                                    if now >= dl {
                                        // Timeout reached - raise TimeoutError
                                        return Err(timeout_error_msg(
                                            vm,
                                            "The read operation timed out".to_string(),
                                        )
                                        .upcast());
                                    }
                                    Some(dl - now)
                                } else {
                                    None // Blocking mode: no timeout
                                };

                                // Wait for socket to be readable
                                let timed_out = self.sock_wait_for_io_with_timeout(
                                    SockWaitKind::Read,
                                    remaining_timeout,
                                    vm,
                                )?;

                                if timed_out {
                                    // Timeout waiting for peer's close_notify
                                    return Err(timeout_error_msg(
                                        vm,
                                        "The read operation timed out".to_string(),
                                    )
                                    .upcast());
                                }

                                // Try to read data from socket
                                let mut conn_guard = self.connection.lock();
                                let conn = match conn_guard.as_mut() {
                                    Some(c) => c,
                                    None => break,
                                };

                                // Read and process any incoming TLS data
                                match self.try_read_close_notify(conn, vm) {
                                    Ok(closed) => {
                                        if closed {
                                            break;
                                        }
                                        // Check again after processing
                                        if self.check_peer_closed(conn, vm)? {
                                            break;
                                        }
                                    }
                                    Err(_) => {
                                        // Socket error - peer likely closed connection
                                        break;
                                    }
                                }
                            }

                            // Shutdown complete
                            *self.shutdown_state.lock() = ShutdownState::Completed;
                            *self.connection.lock() = None;
                            return Ok(self.sock.clone());
                        }
                    }
                }

                // Step 3: Check again if peer has sent close_notify (non-blocking/BIO mode only)
                if !peer_closed {
                    peer_closed = self.check_peer_closed(conn, vm)?;
                }
            }

            drop(conn_guard); // Release lock before returning

            if !peer_closed {
                // Still waiting for peer's close-notify
                // Raise SSLWantReadError to signal app needs to transfer data
                // This is correct for non-blocking sockets and BIO mode
                return Err(create_ssl_want_read_error(vm).upcast());
            }
            // Both close-notify exchanged, shutdown complete
            *self.shutdown_state.lock() = ShutdownState::Completed;

            if is_bio {
                return Ok(vm.ctx.none());
            }
            Ok(self.sock.clone())
        }

        // Helper: Write all pending TLS data (including close_notify) to outgoing buffer/BIO
        fn write_pending_tls(&self, conn: &mut Connection, vm: &VirtualMachine) -> PyResult<()> {
            // First, flush any previously pending TLS output
            // Must succeed before sending new data to maintain order
            self.flush_pending_tls_output(vm, None)?;

            loop {
                if !conn.wants_write() {
                    break;
                }

                let mut buf = vec![0u8; SSL3_RT_MAX_PACKET_SIZE];
                let written = conn
                    .write_tls(&mut buf.as_mut_slice())
                    .map_err(|e| vm.new_os_error(format!("TLS write failed: {e}")))?;

                if written == 0 {
                    break;
                }

                // Send TLS data, saving unsent bytes to pending buffer if needed
                self.send_tls_output(buf[..written].to_vec(), vm)?;
            }

            Ok(())
        }

        // Helper: Try to read incoming data from socket/BIO
        // Returns true if peer closed connection (with or without close_notify)
        fn try_read_close_notify(
            &self,
            conn: &mut Connection,
            vm: &VirtualMachine,
        ) -> PyResult<bool> {
            // In socket mode, peek first to avoid consuming post-TLS cleartext
            // data. During STARTTLS, after close_notify exchange, the socket
            // transitions to cleartext. Without peeking, sock_recv may consume
            // cleartext data meant for the application after unwrap().
            if self.incoming_bio.is_none() {
                return Ok(self.try_read_close_notify_socket(conn, vm));
            }

            // BIO mode: stop at the TLS record boundary so cleartext queued
            // after close_notify remains available to the BIO's owner.
            match self.sock_recv_at_most_one_tls_record(vm) {
                Ok(bytes) => {
                    let data = bytes.as_bytes();

                    if data.is_empty() {
                        if let Some(ref bio) = self.incoming_bio {
                            // BIO mode: check if EOF was signaled via write_eof()
                            let bio_obj: PyObjectRef = bio.clone().into();
                            let eof_attr = bio_obj.get_attr("eof", vm)?;
                            let is_eof = eof_attr.try_to_bool(vm)?;
                            if !is_eof {
                                return Ok(false);
                            }
                        }
                        return Ok(true);
                    }

                    let data_slice: &[u8] = data;
                    let mut cursor = std::io::Cursor::new(data_slice);
                    let _ = conn.read_tls(&mut cursor);
                    let _ = conn.process_new_packets();
                    Ok(false)
                }
                Err(e) => {
                    if is_blocking_io_error(&e, vm) {
                        return Ok(false);
                    }
                    Ok(true)
                }
            }
        }

        /// Socket-mode close_notify reader that respects TLS record boundaries.
        /// Uses MSG_PEEK to inspect data before consuming, preventing accidental
        /// consumption of post-TLS cleartext data during STARTTLS transitions.
        ///
        /// Equivalent to OpenSSL's `SSL_set_read_ahead(ssl, 0)` — rustls has no
        /// such knob, so we enforce record-level reads manually via peek.
        fn try_read_close_notify_socket(&self, conn: &mut Connection, vm: &VirtualMachine) -> bool {
            // Consume at most one TLS record from the socket
            match self.sock_recv_at_most_one_tls_record(vm) {
                Ok(data) => {
                    if data.is_empty() {
                        return true;
                    }

                    let data_slice: &[u8] = data.as_ref();
                    let mut cursor = std::io::Cursor::new(data_slice);
                    let _ = conn.read_tls(&mut cursor);
                    let _ = conn.process_new_packets();
                    false
                }
                Err(e) => {
                    if is_blocking_io_error(&e, vm) {
                        return false;
                    }
                    true
                }
            }
        }

        // Helper: Check if peer has sent close_notify
        fn check_peer_closed(&self, conn: &mut Connection, vm: &VirtualMachine) -> PyResult<bool> {
            // Process any remaining packets and check peer_has_closed
            let io_state = conn
                .process_new_packets()
                .map_err(|e| vm.new_os_error(format!("Failed to process packets: {e}")))?;

            Ok(io_state.peer_has_closed())
        }

        #[pymethod]
        fn shared_ciphers(&self, vm: &VirtualMachine) -> Option<PyListRef> {
            // Return None for client-side sockets
            if !self.server_side {
                return None;
            }

            // Check if handshake completed
            if !*self.handshake_done.lock() {
                return None;
            }

            // Get negotiated cipher suite from rustls
            let conn_guard = self.connection.lock();
            let conn = conn_guard.as_ref()?;

            let suite = conn.negotiated_cipher_suite()?;

            let cipher = cipher::describe(&suite);

            // Return as list with single tuple (name, version, bits)
            let tuple = vm.ctx.new_tuple(vec![
                vm.ctx.new_str(cipher.name).into(),
                vm.ctx.new_str(cipher.protocol).into(),
                vm.ctx.new_int(cipher.bits).into(),
            ]);
            Some(vm.ctx.new_list(vec![tuple.into()]))
        }

        #[pymethod]
        fn verify_client_post_handshake(&self, vm: &VirtualMachine) -> PyResult<()> {
            // TLS 1.3 post-handshake authentication
            // This is only valid for server-side TLS 1.3 connections

            // Check if this is a server-side socket
            if !self.server_side {
                return Err(vm.new_value_error(
                    "Cannot perform post-handshake authentication on client-side socket",
                ));
            }

            // Check if handshake has been completed
            if !*self.handshake_done.lock() {
                return Err(vm.new_value_error(
                    "Handshake must be completed before post-handshake authentication",
                ));
            }

            // Check connection exists and protocol version
            let conn_guard = self.connection.lock();
            if let Some(conn) = conn_guard.as_ref() {
                let version = match conn {
                    Connection::Client(_) => {
                        return Err(vm.new_value_error(
                            "Post-handshake authentication requires server socket",
                        ));
                    }
                    Connection::Server(server) => server.protocol_version(),
                };

                // Post-handshake auth is only available in TLS 1.3
                if version != Some(rustls::ProtocolVersion::TLSv1_3) {
                    // Get SSLError class from ssl module (not _ssl)
                    // ssl.py imports _ssl.SSLError as ssl.SSLError
                    let ssl_mod = vm.import("ssl", 0)?;
                    let ssl_error_class = ssl_mod.get_attr("SSLError", vm)?;

                    // Create SSLError instance with message containing WRONG_SSL_VERSION
                    let msg = "[SSL: WRONG_SSL_VERSION] wrong ssl version";
                    let args = vm.ctx.new_tuple(vec![vm.ctx.new_str(msg).into()]);
                    let exc = ssl_error_class.call((args,), vm)?;

                    return Err(exc
                        .downcast()
                        .map_err(|_| vm.new_type_error("Failed to create SSLError"))?);
                }
            } else {
                return Err(vm.new_value_error("No SSL connection established"));
            }

            // rustls doesn't provide an API for post-handshake authentication.
            // The rustls TLS library does not support requesting client certificates
            // after the initial handshake is completed.
            // Raise SSLError instead of NotImplementedError for compatibility
            Err(vm
                .new_os_subtype_error(
                    PySSLError::class(&vm.ctx).to_owned(),
                    None,
                    "Post-handshake authentication is not supported by the rustls backend. \
                 The rustls TLS library does not provide an API to request client certificates \
                 after the initial handshake. Consider requesting the client certificate \
                 during the initial handshake by setting the appropriate verify_mode before \
                 calling do_handshake().",
                )
                .upcast())
        }

        #[pymethod]
        fn get_channel_binding(
            &self,
            cb_type: OptionalArg<PyUtf8StrRef>,
            vm: &VirtualMachine,
        ) -> PyResult<Option<PyBytesRef>> {
            let cb_type_str = cb_type.as_ref().map_or("tls-unique", |s| s.as_str());

            // rustls doesn't support channel binding (tls-unique, tls-server-end-point, etc.)
            // This is because:
            // 1. tls-unique requires access to TLS Finished messages, which rustls doesn't expose
            // 2. tls-server-end-point requires the server certificate, which we don't track here
            // 3. TLS 1.3 deprecated tls-unique anyway
            //
            // For compatibility, we'll return None (no channel binding available)
            // rather than raising an error

            if cb_type_str != "tls-unique" {
                return Err(vm.new_value_error(format!(
                    "Unsupported channel binding type '{cb_type_str}'",
                )));
            }

            // Return None to indicate channel binding is not available
            // This matches the behavior when the handshake hasn't completed yet
            Ok(None)
        }
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

        fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
            unimplemented!("use slot_new")
        }
    }

    // Clean up SSL socket resources on drop
    impl Drop for PySSLSocket {
        fn drop(&mut self) {
            // Only clear connection state.
            // Do NOT clear pending_tls_output - it may contain data that hasn't
            // been flushed to the socket yet. SSLSocket._real_close() in Python
            // doesn't call shutdown(), so when the socket is closed, pending TLS
            // data would be lost if we clear it here.
            // All fields (Vec, primitives) are automatically freed when the
            // struct is dropped, so explicit clearing is unnecessary.
            let _ = self.connection.lock().take();
        }
    }

    // MemoryBIO - provides in-memory buffer for SSL/TLS I/O
    #[pyattr]
    #[pyclass(name = "MemoryBIO", module = "ssl")]
    #[derive(Debug, PyPayload)]
    struct PyMemoryBIO {
        // Internal buffer
        buffer: PyMutex<Vec<u8>>,
        // EOF flag
        eof: PyRwLock<bool>,
    }

    #[pyclass(with(Constructor), flags(BASETYPE))]
    impl PyMemoryBIO {
        #[pymethod]
        fn read(&self, len: OptionalArg<i32>, vm: &VirtualMachine) -> PyResult<PyBytesRef> {
            let mut buffer = self.buffer.lock();

            if buffer.is_empty() && *self.eof.read() {
                // Return empty bytes at EOF
                return Ok(vm.ctx.new_bytes(vec![]));
            }

            let read_len = match len {
                OptionalArg::Present(n) if n >= 0 => n as usize,
                OptionalArg::Present(n) => {
                    return Err(vm.new_value_error(format!("negative read length: {n}")));
                }
                OptionalArg::Missing => buffer.len(), // Read all available
            };

            let actual_len = read_len.min(buffer.len());
            let data = buffer.drain(..actual_len).collect::<Vec<u8>>();

            Ok(vm.ctx.new_bytes(data))
        }

        #[pymethod]
        fn write(&self, buf: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
            // Check if it's a memoryview and if it's contiguous
            if let Ok(mem_view) = buf.get_attr("c_contiguous", vm) {
                // It's a memoryview, check if contiguous
                let is_contiguous: bool = mem_view.try_to_bool(vm)?;
                if !is_contiguous {
                    return Err(vm.new_buffer_error("non-contiguous buffer is not supported"));
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
            *self.eof.write() = true;
        }

        #[pygetset]
        fn pending(&self) -> i32 {
            self.buffer.lock().len() as i32
        }

        #[pygetset]
        fn eof(&self) -> bool {
            // EOF is true only when buffer is empty AND write_eof has been called
            let pending = self.buffer.lock().len();
            pending == 0 && *self.eof.read()
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
                buffer: PyMutex::new(Vec::new()),
                eof: PyRwLock::new(false),
            })
        }
    }

    // SSLSession - represents a cached SSL session
    // NOTE: This is an EMULATION - actual session data is managed by Rustls internally
    #[derive(Debug, Clone, Copy)]
    enum ClientSessionKind {
        Tls12,
        Tls13,
    }

    #[pyattr]
    #[pyclass(name = "SSLSession", module = "ssl")]
    #[derive(Debug, PyPayload)]
    struct PySSLSession {
        context_identity: Arc<()>,
        client_config: Arc<rustls::ClientConfig>,
        chain_builder: Arc<VerifiedChainBuilder>,
        session_store: Arc<PythonClientSessionStore>,
        server_name: Option<ServerName<'static>>,
        kind: ClientSessionKind,
        // Session ID - synthetic ID generated from metadata (NOT actual TLS session ID)
        session_id: Vec<u8>,
        // Session metadata
        creation_time: std::time::SystemTime,
        // Lifetime in seconds (default 7200 = 2 hours)
        lifetime: u64,
        has_ticket: bool,
    }

    #[pyclass(flags(BASETYPE), with(Comparable))]
    impl PySSLSession {
        #[pygetset]
        fn time(&self) -> i64 {
            // Return session creation time as Unix timestamp
            self.creation_time
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64
        }

        #[pygetset]
        fn timeout(&self) -> i64 {
            // Return session timeout/lifetime in seconds
            self.lifetime as i64
        }

        #[pygetset]
        fn ticket_lifetime_hint(&self) -> i64 {
            // Return ticket lifetime hint (same as timeout for rustls)
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

    // Helper functions

    // OID module already imported at top of _ssl module

    #[derive(FromArgs)]
    struct Txt2ObjArgs {
        txt: PyUtf8StrRef,
        #[pyarg(named, optional)]
        name: OptionalArg<bool>,
    }

    #[pyfunction]
    fn txt2obj(args: Txt2ObjArgs, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let txt = args.txt.as_str();
        let name = args.name.unwrap_or(false);

        // If name=False (default), only accept OID strings
        // If name=True, accept both names and OID strings
        let entry = if txt.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            // Looks like an OID string (starts with digit)
            oid::find_by_oid_string(txt)
        } else if name {
            // name=True: allow shortname/longname lookup
            oid::find_by_name(txt)
        } else {
            // name=False: only OID strings allowed, not names
            None
        };

        let entry = entry.ok_or_else(|| vm.new_value_error(format!("unknown object '{txt}'")))?;

        // Return tuple: (nid, shortname, longname, oid)
        Ok(vm
            .new_tuple((
                vm.ctx.new_int(entry.nid),
                vm.ctx.new_str(entry.short_name),
                vm.ctx.new_str(entry.long_name),
                // OpenSSL names some objects without giving them an OID, and
                // `nid2obj` reports the fourth field as None for those.
                entry
                    .oid_string()
                    .map_or_else(|| vm.ctx.none(), |oid| vm.ctx.new_str(oid).into()),
            ))
            .into())
    }

    #[pyfunction]
    fn nid2obj(nid: i32, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let entry = oid::find_by_nid(nid)
            .ok_or_else(|| vm.new_value_error(format!("unknown NID {nid}")))?;

        // Return tuple: (nid, shortname, longname, oid)
        Ok(vm
            .new_tuple((
                vm.ctx.new_int(entry.nid),
                vm.ctx.new_str(entry.short_name),
                vm.ctx.new_str(entry.long_name),
                // OpenSSL names some objects without giving them an OID, and
                // `nid2obj` reports the fourth field as None for those.
                entry
                    .oid_string()
                    .map_or_else(|| vm.ctx.none(), |oid| vm.ctx.new_str(oid).into()),
            ))
            .into())
    }

    #[pyfunction]
    fn get_default_verify_paths(vm: &VirtualMachine) -> PyObjectRef {
        // Lib/ssl.py expects: (openssl_cafile_env, openssl_cafile, openssl_capath_env, openssl_capath)
        let (default_cafile, default_capath) =
            rustpython_host_env::native_certs::default_verify_paths();

        let tuple = vm.ctx.new_tuple(vec![
            vm.ctx.new_str("SSL_CERT_FILE").into(), // openssl_cafile_env
            vm.ctx.new_str(default_cafile).into(),  // openssl_cafile
            vm.ctx.new_str("SSL_CERT_DIR").into(),  // openssl_capath_env
            vm.ctx.new_str(default_capath).into(),  // openssl_capath
        ]);

        tuple.into()
    }

    #[pyfunction]
    fn RAND_status() -> i32 {
        1 // The configured rustls provider supplies cryptographic randomness.
    }

    #[pyfunction]
    fn RAND_add(_string: PyObjectRef, _entropy: f64) {
        // No-op: the configured rustls provider handles its own entropy.
        // Accept any type (str, bytes, bytearray)
    }

    #[pyfunction]
    fn RAND_bytes(n: i32, vm: &VirtualMachine) -> PyResult<PyBytesRef> {
        // Validate n is not negative
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

    #[pyfunction]
    fn RAND_pseudo_bytes(n: i32, vm: &VirtualMachine) -> PyResult<(PyBytesRef, bool)> {
        // Rustls providers expose cryptographically strong random bytes.
        let bytes = RAND_bytes(n, vm)?;
        Ok((bytes, true))
    }

    /// Test helper to decode a certificate from a file path
    ///
    /// This is a simplified wrapper around cert_der_to_dict_helper that handles
    /// file reading and PEM/DER auto-detection. Used by test suite.
    #[pyfunction]
    fn _test_decode_cert(path: PyUtf8StrRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        // Read certificate file
        let path_str = path.as_str();
        let cert_data = rustpython_host_env::fs::read(path_str).map_err(|e| {
            vm.new_os_error(format!("Failed to read certificate file {path_str}: {e}"))
        })?;

        // Auto-detect PEM vs DER format
        let cert_der = if cert_data
            .windows(27)
            .any(|w| w == b"-----BEGIN CERTIFICATE-----")
        {
            // Parse PEM format
            let mut cursor = std::io::Cursor::new(&cert_data);
            rustls_pemfile::certs(&mut cursor)
                .find_map(|r| r.ok())
                .ok_or_else(|| vm.new_value_error("No valid certificate found in PEM file"))?
                .to_vec()
        } else {
            // Assume DER format
            cert_data
        };

        // Reuse the comprehensive helper function
        cert::cert_der_to_dict_helper(vm, &cert_der)
    }

    #[pyfunction]
    fn DER_cert_to_PEM_cert(der_cert: ArgBytesLike, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        let der_bytes = der_cert.borrow_buf();
        let bytes_slice: &[u8] = der_bytes.as_ref();

        // Use pem-rfc7468 for RFC 7468 compliant PEM encoding
        let pem_str = encode_string("CERTIFICATE", LineEnding::LF, bytes_slice)
            .map_err(|e| vm.new_value_error(format!("PEM encoding failed: {e}")))?;

        Ok(vm.ctx.new_str(pem_str))
    }

    #[pyfunction]
    fn PEM_cert_to_DER_cert(pem_cert: PyUtf8StrRef, vm: &VirtualMachine) -> PyResult<PyBytesRef> {
        // Parse PEM format
        let mut cursor = std::io::Cursor::new(pem_cert.as_bytes());
        let mut certs = rustls_pemfile::certs(&mut cursor);

        if let Some(Ok(cert)) = certs.next() {
            Ok(vm.ctx.new_bytes(cert.to_vec()))
        } else {
            Err(vm.new_value_error("Failed to parse PEM certificate"))
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
        let certs = rustpython_host_env::cert_store::enum_certificates(store_name_str);
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
        let crls = rustpython_host_env::cert_store::enum_crls(store_name_str).map_err(|_| {
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

    // Certificate type for SSL module (pure Rust implementation)
    #[pyattr]
    #[pyclass(module = "_ssl", name = "Certificate")]
    #[derive(Debug, PyPayload)]
    pub(super) struct PySSLCertificate {
        // Store the raw DER bytes
        der_bytes: Vec<u8>,
    }

    impl PySSLCertificate {
        // Parse the certificate lazily
        fn parse(&self) -> Result<x509_parser::certificate::X509Certificate<'_>, String> {
            match x509_parser::parse_x509_certificate(&self.der_bytes) {
                Ok((_, cert)) => Ok(cert),
                Err(e) => Err(format!("Failed to parse certificate: {e}")),
            }
        }
    }

    #[pyclass(
        flags(IMMUTABLETYPE, DISALLOW_INSTANTIATION),
        with(Comparable, Hashable, Representable)
    )]
    impl PySSLCertificate {
        #[pymethod]
        fn public_bytes(
            &self,
            format: OptionalArg<i32>,
            vm: &VirtualMachine,
        ) -> PyResult<PyObjectRef> {
            let format = format.unwrap_or(ENCODING_PEM);

            match format {
                x if x == ENCODING_DER => {
                    // Return DER bytes directly
                    Ok(vm.ctx.new_bytes(self.der_bytes.clone()).into())
                }
                x if x == ENCODING_PEM => {
                    // Convert DER to PEM using RFC 7468 compliant encoding
                    let pem_str = encode_string("CERTIFICATE", LineEnding::LF, &self.der_bytes)
                        .map_err(|e| vm.new_value_error(format!("PEM encoding failed: {e}")))?;
                    Ok(vm.ctx.new_str(pem_str).into())
                }
                _ => Err(vm.new_value_error("Unsupported format")),
            }
        }

        #[pymethod]
        fn get_info(&self, vm: &VirtualMachine) -> PyResult {
            let cert = self.parse().map_err(|e| vm.new_value_error(e))?;
            cert::cert_to_dict(vm, &cert)
        }
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
                    Ok((zelf.der_bytes == other_cert.der_bytes).into())
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
            zelf.der_bytes.hash(&mut hasher);
            Ok(hasher.finish() as PyHash)
        }
    }

    // Implement Representable trait for PySSLCertificate
    impl Representable for PySSLCertificate {
        #[inline]
        fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            // Try to parse and show subject
            match zelf.parse() {
                Ok(cert) => {
                    let subject = cert.subject();
                    // Get CN if available
                    let cn = subject
                        .iter_common_name()
                        .next()
                        .and_then(|attr| attr.as_str().ok())
                        .unwrap_or("Unknown");
                    Ok(format!("<Certificate(subject=CN={cn})>"))
                }
                Err(_) => Ok("<Certificate(invalid)>".to_owned()),
            }
        }
    }
}
