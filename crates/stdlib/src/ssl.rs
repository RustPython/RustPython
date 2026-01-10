// spell-checker: ignore ssleof aesccm aesgcm getblocking setblocking ENDTLS TLSEXT

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

// OpenSSL compatibility layer (abstracts rustls operations)
mod compat;

// SSL exception types (shared with openssl backend)
mod error;

pub(crate) use _ssl::make_module;

#[allow(non_snake_case)]
#[allow(non_upper_case_globals)]
#[pymodule(with(error::ssl_error))]
mod _ssl {
    use crate::{
        common::{
            hash::PyHash,
            lock::{PyMutex, PyRwLock},
        },
        socket::{PySocket, SelectKind, sock_select, timeout_error_msg},
        vm::{
            AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject,
            VirtualMachine,
            builtins::{PyBaseExceptionRef, PyBytesRef, PyListRef, PyStrRef, PyType, PyTypeRef},
            convert::IntoPyException,
            function::{ArgBytesLike, ArgMemoryBuffer, FuncArgs, OptionalArg, PyComparisonValue},
            stdlib::warnings,
            types::{Comparable, Constructor, Hashable, PyComparisonOp, Representable},
        },
    };

    // Import error types used in this module (others are exposed via pymodule(with(...)))
    use super::error::{
        PySSLEOFError, PySSLError, create_ssl_want_read_error, create_ssl_want_write_error,
    };
    use alloc::sync::Arc;
    use core::{
        sync::atomic::{AtomicUsize, Ordering},
        time::Duration,
    };
    use std::{collections::HashMap, time::SystemTime};

    // Rustls imports
    use parking_lot::{Mutex as ParkingMutex, RwLock as ParkingRwLock};
    use pem_rfc7468::{LineEnding, encode_string};
    use rustls::{
        ClientConfig, ClientConnection, RootCertStore, ServerConfig, ServerConnection,
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

    // Import OID module
    use super::oid;

    // Import compat module (OpenSSL compatibility layer)
    use super::compat::{
        ClientConfigOptions, MultiCertResolver, ProtocolSettings, ServerConfigOptions, SslError,
        TlsConnection, create_client_config, create_server_config, curve_name_to_kx_group,
        extract_cipher_info, get_cipher_encryption_desc, is_blocking_io_error,
        normalize_cipher_name, ssl_do_handshake,
    };

    // Type aliases for better readability
    // Additional type alias for certificate/key pairs (SessionCache and SniCertName defined below)

    /// Certificate and private key pair used in SSL contexts
    type CertKeyPair = (Arc<CertifiedKey>, PrivateKeyDer<'static>);

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
    const SSL3_RT_MAX_PLAIN_LENGTH: usize = 16384;
    // SSL session cache size (common practice, similar to OpenSSL defaults)
    const SSL_SESSION_CACHE_SIZE: usize = 256;

    // Certificate verification modes
    #[pyattr]
    const CERT_NONE: i32 = 0;
    #[pyattr]
    const CERT_OPTIONAL: i32 = 1;
    #[pyattr]
    const CERT_REQUIRED: i32 = 2;

    // Certificate requirements
    #[pyattr]
    const VERIFY_DEFAULT: i32 = 0;
    #[pyattr]
    const VERIFY_CRL_CHECK_LEAF: i32 = 4;
    #[pyattr]
    const VERIFY_CRL_CHECK_CHAIN: i32 = 12;
    #[pyattr]
    const VERIFY_X509_STRICT: i32 = 32;
    #[pyattr]
    const VERIFY_ALLOW_PROXY_CERTS: i32 = 64;
    #[pyattr]
    const VERIFY_X509_TRUSTED_FIRST: i32 = 32768;
    #[pyattr]
    const VERIFY_X509_PARTIAL_CHAIN: i32 = 0x80000;

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

    // Version info - reporting as OpenSSL 3.3.0 for compatibility
    #[pyattr]
    const OPENSSL_VERSION_NUMBER: i32 = 0x30300000; // OpenSSL 3.3.0 (808452096)
    #[pyattr]
    const OPENSSL_VERSION: &str = "OpenSSL 3.3.0 (rustls/0.23)";
    #[pyattr]
    const OPENSSL_VERSION_INFO: (i32, i32, i32, i32, i32) = (3, 3, 0, 0, 15); // 3.3.0 release
    #[pyattr]
    const _OPENSSL_API_VERSION: (i32, i32, i32, i32, i32) = (3, 3, 0, 0, 15); // 3.3.0 release

    // Default cipher list for rustls - using modern secure ciphers
    #[pyattr]
    const _DEFAULT_CIPHERS: &str =
        "TLS_AES_256_GCM_SHA384:TLS_AES_128_GCM_SHA256:TLS_CHACHA20_POLY1305_SHA256";

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

        if hostname.contains('\0') {
            return Err(vm.new_type_error("embedded null character"));
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
        #[allow(dead_code)]
        server_name: String,
        session_id: Vec<u8>,
        creation_time: SystemTime,
        lifetime: u64,
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

    // Generate a synthetic session ID from server name and timestamp
    // NOTE: This is NOT the actual TLS session ID, just a unique identifier
    fn generate_session_id_from_metadata(server_name: &str, time: &SystemTime) -> Vec<u8> {
        let mut hasher = Sha256::new();
        hasher.update(server_name.as_bytes());
        hasher.update(
            time.duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                .to_le_bytes(),
        );
        hasher.finalize()[..16].to_vec()
    }

    // Custom ClientSessionStore that tracks session metadata for Python access
    // NOTE: This wraps ClientSessionMemoryCache and records metadata when sessions are stored
    #[derive(Debug)]
    struct PythonClientSessionStore {
        inner: Arc<ClientSessionMemoryCache>,
        session_cache: SessionCache,
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
            let creation_time = SystemTime::now();
            let server_name_str = server_name.to_str();
            let session_data = SessionData {
                server_name: server_name_str.as_ref().to_string(),
                session_id: generate_session_id_from_metadata(
                    server_name_str.as_ref(),
                    &creation_time,
                ),
                creation_time,
                lifetime: 7200, // TLS 1.2 default session lifetime
            };

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
            let creation_time = SystemTime::now();
            let server_name_str = server_name.to_str();
            let session_data = SessionData {
                server_name: server_name_str.to_string(),
                session_id: generate_session_id_from_metadata(
                    server_name_str.as_ref(),
                    &creation_time,
                ),
                creation_time,
                lifetime: 7200, // Default TLS 1.3 ticket lifetime (Rustls uses this)
            };

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

    /// Parse OpenSSL cipher string to rustls SupportedCipherSuite list
    ///
    /// Supports patterns like:
    /// - "AES128" → filters for AES_128
    /// - "AES256" → filters for AES_256
    /// - "AES128:AES256" → both
    /// - "ECDHE+AESGCM" → ECDHE AND AESGCM (both conditions must match)
    /// - "ALL" or "DEFAULT" → all available
    /// - "!MD5" → exclusion (ignored, rustls doesn't support weak ciphers anyway)
    fn parse_cipher_string(cipher_str: &str) -> Result<Vec<rustls::SupportedCipherSuite>, String> {
        use rustls::crypto::aws_lc_rs::ALL_CIPHER_SUITES;

        if cipher_str.is_empty() {
            return Err("No cipher can be selected".to_string());
        }

        let all_suites = ALL_CIPHER_SUITES;
        let mut selected = Vec::new();

        for part in cipher_str.split(':') {
            let part = part.trim();

            // Skip exclusions (rustls doesn't support these)
            if part.starts_with('!') {
                continue;
            }

            // Skip priority markers starting with +
            if part.starts_with('+') {
                continue;
            }

            // Match pattern
            match part {
                "ALL" | "DEFAULT" | "HIGH" => {
                    // Add all available cipher suites
                    selected.extend_from_slice(all_suites);
                }
                _ => {
                    // Check if this is a compound pattern with + (AND condition)
                    // e.g., "ECDHE+AESGCM" means ECDHE AND AESGCM
                    let patterns: Vec<&str> = part.split('+').collect();

                    let mut found_any = false;
                    for suite in all_suites {
                        let name = format!("{:?}", suite.suite());

                        // Check if all patterns match (AND condition)
                        let matches = patterns.iter().all(|&pattern| {
                            // Handle common OpenSSL pattern variations
                            if pattern.contains("AES128") {
                                name.contains("AES_128")
                            } else if pattern.contains("AES256") {
                                name.contains("AES_256")
                            } else if pattern == "AESGCM" {
                                // AESGCM: AES with GCM mode
                                name.contains("AES") && name.contains("GCM")
                            } else if pattern == "AESCCM" {
                                // AESCCM: AES with CCM mode
                                name.contains("AES") && name.contains("CCM")
                            } else if pattern == "CHACHA20" {
                                name.contains("CHACHA20")
                            } else if pattern == "ECDHE" {
                                name.contains("ECDHE")
                            } else if pattern == "DHE" {
                                // DHE but not ECDHE
                                name.contains("DHE") && !name.contains("ECDHE")
                            } else if pattern == "ECDH" {
                                // ECDH but not ECDHE
                                name.contains("ECDH") && !name.contains("ECDHE")
                            } else if pattern == "DH" {
                                // DH but not DHE or ECDH
                                name.contains("DH")
                                    && !name.contains("DHE")
                                    && !name.contains("ECDH")
                            } else if pattern == "RSA" {
                                name.contains("RSA")
                            } else if pattern == "AES" {
                                name.contains("AES")
                            } else if pattern == "ECDSA" {
                                name.contains("ECDSA")
                            } else {
                                // Direct substring match for other patterns
                                name.contains(pattern)
                            }
                        });

                        if matches {
                            selected.push(*suite);
                            found_any = true;
                        }
                    }

                    if !found_any {
                        // No matching cipher suite found - warn but continue
                    }
                }
            }
        }

        // Remove duplicates
        selected.dedup_by_key(|s| s.suite());

        if selected.is_empty() {
            Err("No cipher can be selected".to_string())
        } else {
            Ok(selected)
        }
    }

    // SSLContext - manages TLS configuration
    #[pyattr]
    #[pyclass(name = "_SSLContext", module = "ssl", traverse)]
    #[derive(Debug, PyPayload)]
    struct PySSLContext {
        #[pytraverse(skip)]
        protocol: i32,
        #[pytraverse(skip)]
        check_hostname: PyRwLock<bool>,
        #[pytraverse(skip)]
        verify_mode: PyRwLock<i32>,
        #[pytraverse(skip)]
        verify_flags: PyRwLock<i32>,
        // Rustls configuration (built lazily)
        #[allow(dead_code)]
        #[pytraverse(skip)]
        client_config: PyRwLock<Option<Arc<ClientConfig>>>,
        #[allow(dead_code)]
        #[pytraverse(skip)]
        server_config: PyRwLock<Option<Arc<ServerConfig>>>,
        // Certificate store
        #[pytraverse(skip)]
        root_certs: PyRwLock<RootCertStore>,
        // Store full CA certificates for get_ca_certs()
        // RootCertStore only keeps TrustAnchors, not full certificates
        #[pytraverse(skip)]
        ca_certs_der: PyRwLock<Vec<Vec<u8>>>,
        // Store CA certificates from capath for lazy loading simulation
        // (CPython only returns these in get_ca_certs() after they're used in handshake)
        #[pytraverse(skip)]
        capath_certs_der: PyRwLock<Vec<Vec<u8>>>,
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
        #[allow(dead_code)]
        #[pytraverse(skip)]
        options: PyRwLock<i32>,
        // ALPN protocols
        #[allow(dead_code)]
        #[pytraverse(skip)]
        alpn_protocols: PyRwLock<Vec<Vec<u8>>>,
        // ALPN strict matching flag
        // When false (default), mimics OpenSSL behavior: no ALPN negotiation failure
        // When true, requires ALPN match (Rustls default behavior)
        #[allow(dead_code)]
        #[pytraverse(skip)]
        require_alpn_match: PyRwLock<bool>,
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
        // Rustls session store for actual TLS session resumption
        #[pytraverse(skip)]
        rustls_session_store: Arc<PythonClientSessionStore>,
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
    }

    #[derive(FromArgs)]
    struct WrapSocketArgs {
        sock: PyObjectRef,
        server_side: bool,
        #[pyarg(positional, optional)]
        server_hostname: OptionalArg<Option<PyStrRef>>,
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
        server_hostname: OptionalArg<Option<PyStrRef>>,
        #[pyarg(named, optional)]
        owner: OptionalArg<PyObjectRef>,
        #[pyarg(named, optional)]
        session: OptionalArg<PyObjectRef>,
    }

    #[derive(FromArgs)]
    struct LoadVerifyLocationsArgs {
        #[pyarg(any, optional)]
        cafile: OptionalArg<Option<PyObjectRef>>,
        #[pyarg(any, optional)]
        capath: OptionalArg<Option<PyObjectRef>>,
        #[pyarg(any, optional)]
        cadata: OptionalArg<PyObjectRef>,
    }

    #[derive(FromArgs)]
    struct LoadCertChainArgs {
        #[pyarg(any)]
        certfile: PyObjectRef,
        #[pyarg(any, optional)]
        keyfile: OptionalArg<Option<PyObjectRef>>,
        #[pyarg(any, optional)]
        password: OptionalArg<PyObjectRef>,
    }

    #[derive(FromArgs)]
    struct GetCertArgs {
        #[pyarg(any, optional)]
        binary_form: OptionalArg<bool>,
    }

    #[pyclass(with(Constructor), flags(BASETYPE))]
    impl PySSLContext {
        // Helper method to convert DER certificate bytes to Python dict
        fn cert_der_to_dict(&self, vm: &VirtualMachine, cert_der: &[u8]) -> PyResult<PyObjectRef> {
            cert::cert_der_to_dict_helper(vm, cert_der)
        }

        #[pymethod]
        fn __repr__(&self) -> String {
            format!("<SSLContext(protocol={})>", self.protocol)
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
                return Err(vm.new_overflow_error("options must be non-negative".to_owned()));
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
                warnings::warn(
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
                let password_from_callable = if let Ok(pwd_str) =
                    PyStrRef::try_from_object(vm, pwd_result.clone())
                {
                    pwd_str.as_str().to_owned()
                } else if let Ok(pwd_bytes_like) = ArgBytesLike::try_from_object(vm, pwd_result) {
                    String::from_utf8(pwd_bytes_like.borrow_buf().to_vec()).map_err(|_| {
                        vm.new_type_error(
                            "password callback returned invalid UTF-8 bytes".to_owned(),
                        )
                    })?
                } else {
                    return Err(vm.new_type_error(
                        "password callback must return a string or bytes".to_owned(),
                    ));
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
            let signing_key =
                rustls::crypto::aws_lc_rs::sign::any_supported_type(&key).map_err(|_| {
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
            let has_cadata = matches!(&args.cadata, OptionalArg::Present(obj) if !vm.is_none(obj));

            if !has_cafile && !has_capath && !has_cadata {
                return Err(
                    vm.new_type_error("cafile, capath and cadata cannot be all omitted".to_owned())
                );
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

            let cadata_parsed = if let OptionalArg::Present(ref cadata_obj) = args.cadata
                && !vm.is_none(cadata_obj)
            {
                let is_string = PyStrRef::try_from_object(vm, cadata_obj.clone()).is_ok();
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

            // Load from directory (don't add to ca_certs_der)
            if let Some(ref dir_path) = capath_dir {
                let stats = self.load_certs_from_dir_helper(&mut root_store, dir_path, vm)?;
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
        /// Returns true if certificates were successfully loaded.
        ///
        /// We use Python's os.environ instead of Rust's std::env
        /// because Python code can modify os.environ at runtime (e.g.,
        /// `os.environ['SSL_CERT_FILE'] = '/path'`), but rustls-native-certs uses
        /// std::env which only sees the process environment at startup.
        fn try_load_from_python_environ(
            &self,
            loader: &mut cert::CertLoader<'_>,
            vm: &VirtualMachine,
        ) -> PyResult<bool> {
            use std::path::Path;

            let os_module = vm.import("os", 0)?;
            let environ = os_module.get_attr("environ", vm)?;

            // Try SSL_CERT_FILE first
            if let Ok(cert_file) = Self::get_env_path(&environ, "SSL_CERT_FILE", vm)
                && Path::new(&cert_file).exists()
                && let Ok(stats) = loader.load_from_file(&cert_file)
            {
                self.update_cert_stats(stats);
                return Ok(true);
            }

            // Try SSL_CERT_DIR (only if SSL_CERT_FILE didn't work)
            if let Ok(cert_dir) = Self::get_env_path(&environ, "SSL_CERT_DIR", vm)
                && Path::new(&cert_dir).is_dir()
                && let Ok(stats) = loader.load_from_dir(&cert_dir)
            {
                self.update_cert_stats(stats);
                return Ok(true);
            }

            Ok(false)
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
                // Windows: Use schannel to load from both ROOT and CA stores
                use schannel::cert_store::CertStore;

                let store_names = ["ROOT", "CA"];
                let open_fns = [CertStore::open_current_user, CertStore::open_local_machine];

                for store_name in store_names {
                    for open_fn in &open_fns {
                        if let Ok(cert_store) = open_fn(store_name) {
                            for cert_ctx in cert_store.certs() {
                                let der_bytes = cert_ctx.to_der();
                                let cert =
                                    rustls::pki_types::CertificateDer::from(der_bytes.to_vec());
                                let is_ca = cert::is_ca_certificate(cert.as_ref());
                                if store.add(cert).is_ok() {
                                    *self.x509_cert_count.write() += 1;
                                    if is_ca {
                                        *self.ca_cert_count.write() += 1;
                                    }
                                }
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
                let result = rustls_native_certs::load_native_certs();

                // Load successfully found certificates
                for cert in result.certs {
                    let is_ca = cert::is_ca_certificate(cert.as_ref());
                    if store.add(cert).is_ok() {
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

                let mut lazy_ca_certs = Vec::new();
                let mut loader = cert::CertLoader::new(&mut store, &mut lazy_ca_certs);
                let _ = self.try_load_from_python_environ(&mut loader, vm)?;
            }

            #[cfg(not(windows))]
            {
                // Non-Windows: Try env vars first; only fallback to system certs if not set
                // see: test_load_default_certs_env
                let mut lazy_ca_certs = Vec::new();
                let mut loader = cert::CertLoader::new(&mut store, &mut lazy_ca_certs);
                let loaded = self.try_load_from_python_environ(&mut loader, vm)?;

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
        fn set_ciphers(&self, ciphers: PyStrRef, vm: &VirtualMachine) -> PyResult<()> {
            let cipher_str = ciphers.as_str();

            // Parse cipher string and store selected ciphers
            let selected_ciphers = parse_cipher_string(cipher_str).map_err(|e| {
                vm.new_os_subtype_error(PySSLError::class(&vm.ctx).to_owned(), None, e)
                    .upcast()
            })?;

            // Store in context
            *self.selected_ciphers.write() = Some(selected_ciphers);

            Ok(())
        }

        #[pymethod]
        fn get_ciphers(&self, vm: &VirtualMachine) -> PyResult<PyListRef> {
            // Dynamically generate cipher list from rustls ALL_CIPHER_SUITES
            // This automatically includes all cipher suites supported by the current rustls version
            use rustls::crypto::aws_lc_rs::ALL_CIPHER_SUITES;

            let cipher_list = ALL_CIPHER_SUITES
                .iter()
                .map(|suite| {
                    // Extract cipher information using unified helper
                    let cipher_info = extract_cipher_info(suite);

                    // Convert to OpenSSL-style name
                    // e.g., "TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256" -> "ECDHE-RSA-AES128-GCM-SHA256"
                    let openssl_name = normalize_cipher_name(&cipher_info.name);

                    // Determine key exchange and auth methods
                    let (kx, auth) = if cipher_info.protocol == "TLSv1.3" {
                        // TLS 1.3 doesn't distinguish - all use modern algos
                        ("any", "any")
                    } else if cipher_info.name.contains("ECDHE") {
                        // TLS 1.2 with ECDHE
                        let auth = if cipher_info.name.contains("ECDSA") {
                            "ECDSA"
                        } else if cipher_info.name.contains("RSA") {
                            "RSA"
                        } else {
                            "any"
                        };
                        ("ECDH", auth)
                    } else {
                        ("any", "any")
                    };

                    // Build description string
                    // Format: "{name} {protocol} Kx={kx} Au={auth} Enc={enc} Mac={mac}"
                    let enc = get_cipher_encryption_desc(&openssl_name);

                    let description = format!(
                        "{} {} Kx={} Au={} Enc={} Mac=AEAD",
                        openssl_name, cipher_info.protocol, kx, auth, enc
                    );

                    // Create cipher dict
                    let dict = vm.ctx.new_dict();
                    dict.set_item("name", vm.ctx.new_str(openssl_name).into(), vm)
                        .unwrap();
                    dict.set_item("protocol", vm.ctx.new_str(cipher_info.protocol).into(), vm)
                        .unwrap();
                    dict.set_item("id", vm.ctx.new_int(0).into(), vm).unwrap(); // Placeholder ID
                    dict.set_item("strength_bits", vm.ctx.new_int(cipher_info.bits).into(), vm)
                        .unwrap();
                    dict.set_item("alg_bits", vm.ctx.new_int(cipher_info.bits).into(), vm)
                        .unwrap();
                    dict.set_item("description", vm.ctx.new_str(description).into(), vm)
                        .unwrap();
                    dict.into()
                })
                .collect::<Vec<_>>();

            Ok(PyListRef::from(vm.ctx.new_list(cipher_list)))
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
                return Err(vm.new_type_error("DH params filepath cannot be None".to_owned()));
            }

            // Validate filepath is str or bytes
            let path_str = if let Ok(s) = PyStrRef::try_from_object(vm, filepath.clone()) {
                s.as_str().to_owned()
            } else if let Ok(b) = ArgBytesLike::try_from_object(vm, filepath) {
                String::from_utf8(b.borrow_buf().to_vec())
                    .map_err(|_| vm.new_value_error("Invalid path encoding".to_owned()))?
            } else {
                return Err(vm.new_type_error("DH params filepath must be str or bytes".to_owned()));
            };

            // Check if file exists
            if !std::path::Path::new(&path_str).exists() {
                // Create FileNotFoundError with errno=ENOENT (2)
                let exc = vm.new_os_subtype_error(
                    vm.ctx.exceptions.file_not_found_error.to_owned(),
                    Some(2), // errno = ENOENT (2)
                    "No such file or directory",
                );
                // Set filename attribute
                let _ = exc
                    .as_object()
                    .set_attr("filename", vm.ctx.new_str(path_str.clone()), vm);
                return Err(exc.upcast());
            }

            // Validate that the file contains DH parameters
            // Read the file and check for DH PARAMETERS header
            let contents =
                std::fs::read_to_string(&path_str).map_err(|e| vm.new_os_error(e.to_string()))?;

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
                return Err(vm.new_type_error("ECDH curve name cannot be None".to_owned()));
            }

            // Validate name is str or bytes
            let curve_name = if let Ok(s) = PyStrRef::try_from_object(vm, name.clone()) {
                s.as_str().to_owned()
            } else if let Ok(b) = ArgBytesLike::try_from_object(vm, name) {
                String::from_utf8(b.borrow_buf().to_vec())
                    .map_err(|_| vm.new_value_error("Invalid curve name encoding".to_owned()))?
            } else {
                return Err(vm.new_type_error("ECDH curve name must be str or bytes".to_owned()));
            };

            // Validate curve name (common curves for compatibility)
            // rustls supports: X25519, secp256r1 (prime256v1), secp384r1
            let valid_curves = [
                "prime256v1",
                "secp256r1",
                "prime384v1",
                "secp384r1",
                "prime521v1",
                "secp521r1",
                "X25519",
                "x25519",
                "x448", // For future compatibility
            ];

            if !valid_curves.contains(&curve_name.as_str()) {
                return Err(vm.new_value_error(format!("unknown curve name '{curve_name}'")));
            }

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
            // Convert server_hostname to Option<String>
            // Handle both missing argument and None value
            let hostname = match args.server_hostname.into_option().flatten() {
                Some(hostname_str) => {
                    let hostname = hostname_str.as_str();

                    // Validate hostname
                    if hostname.is_empty() {
                        return Err(vm.new_value_error("server_hostname cannot be an empty string"));
                    }

                    // Check if it starts with a dot
                    if hostname.starts_with('.') {
                        return Err(vm.new_value_error("server_hostname cannot start with a dot"));
                    }

                    // IP addresses are allowed
                    // SNI will not be sent for IP addresses

                    // Check for NULL bytes
                    if hostname.contains('\0') {
                        return Err(vm.new_type_error("embedded null character"));
                    }

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
                context: PyRwLock::new(zelf),
                server_side: args.server_side,
                server_hostname: PyRwLock::new(hostname),
                connection: PyMutex::new(None),
                handshake_done: PyMutex::new(false),
                session_was_reused: PyMutex::new(false),
                owner: PyRwLock::new(args.owner.into_option()),
                // Filter out Python None objects - only store actual SSLSession objects
                session: PyRwLock::new(args.session.into_option().filter(|s| !vm.is_none(s))),
                verified_chain: PyRwLock::new(None),
                incoming_bio: None,
                outgoing_bio: None,
                sni_state: PyRwLock::new(None),
                pending_context: PyRwLock::new(None),
                client_hello_buffer: PyMutex::new(None),
                shutdown_state: PyMutex::new(ShutdownState::NotStarted),
                pending_tls_output: PyMutex::new(Vec::new()),
                deferred_cert_error: Arc::new(ParkingRwLock::new(None)),
            };

            // Create PyRef with correct type
            let ssl_socket_ref = ssl_socket
                .into_ref_with_type(vm, vm.class("_ssl", "_SSLSocket"))
                .map_err(|_| vm.new_type_error("Failed to create SSLSocket"))?;

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
                sock: vm.ctx.none(), // No socket in BIO mode
                context: PyRwLock::new(zelf),
                server_side,
                server_hostname: PyRwLock::new(hostname),
                connection: PyMutex::new(None),
                handshake_done: PyMutex::new(false),
                session_was_reused: PyMutex::new(false),
                owner: PyRwLock::new(args.owner.into_option()),
                // Filter out Python None objects - only store actual SSLSession objects
                session: PyRwLock::new(args.session.into_option().filter(|s| !vm.is_none(s))),
                verified_chain: PyRwLock::new(None),
                incoming_bio: Some(args.incoming),
                outgoing_bio: Some(args.outgoing),
                sni_state: PyRwLock::new(None),
                pending_context: PyRwLock::new(None),
                client_hello_buffer: PyMutex::new(None),
                shutdown_state: PyMutex::new(ShutdownState::NotStarted),
                pending_tls_output: PyMutex::new(Vec::new()),
                deferred_cert_error: Arc::new(ParkingRwLock::new(None)),
            };

            let ssl_socket_ref = ssl_socket
                .into_ref_with_type(vm, vm.class("_ssl", "_SSLSocket"))
                .map_err(|_| vm.new_type_error("Failed to create SSLSocket"))?;

            Ok(ssl_socket_ref)
        }

        // Helper functions (private):

        /// Parse path argument (str or bytes) to string
        fn parse_path_arg(arg: &PyObject, vm: &VirtualMachine) -> PyResult<String> {
            if let Ok(s) = PyStrRef::try_from_object(vm, arg.to_owned()) {
                Ok(s.as_str().to_owned())
            } else if let Ok(b) = ArgBytesLike::try_from_object(vm, arg.to_owned()) {
                String::from_utf8(b.borrow_buf().to_vec())
                    .map_err(|_| vm.new_value_error("path contains invalid UTF-8".to_owned()))
            } else {
                Err(vm.new_type_error("path should be a str or bytes".to_owned()))
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
                    // Try string first
                    if let Ok(pwd_str) = PyStrRef::try_from_object(vm, p.clone()) {
                        Ok((Some(pwd_str.as_str().to_owned()), None))
                    }
                    // Try bytes-like
                    else if let Ok(pwd_bytes_like) = ArgBytesLike::try_from_object(vm, p.clone())
                    {
                        let pwd = String::from_utf8(pwd_bytes_like.borrow_buf().to_vec()).map_err(
                            |_| vm.new_type_error("password bytes must be valid UTF-8".to_owned()),
                        )?;
                        Ok((Some(pwd), None))
                    }
                    // Try callable
                    else if p.is_callable() {
                        Ok((None, Some(p.clone())))
                    } else {
                        Err(vm.new_type_error(
                            "password should be a string, bytes, or callable".to_owned(),
                        ))
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

        /// Helper: Load certificates from directory into existing store
        fn load_certs_from_dir_helper(
            &self,
            root_store: &mut RootCertStore,
            path: &str,
            vm: &VirtualMachine,
        ) -> PyResult<cert::CertStats> {
            // Load certs and store them in capath_certs_der for lazy loading simulation
            // (CPython only returns these in get_ca_certs() after they're used in handshake)
            let mut capath_certs = Vec::new();
            let mut loader = cert::CertLoader::new(root_store, &mut capath_certs);
            let stats = loader
                .load_from_dir(path)
                .map_err(|e| e.into_pyexception(vm))?;

            // Store loaded certs for potential tracking after handshake
            *self.capath_certs_der.write() = capath_certs;

            Ok(stats)
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
            let data = std::fs::read(path).map_err(|e| match e.kind() {
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
        fn parse_cadata_arg(&self, arg: &PyObject, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            if let Ok(s) = PyStrRef::try_from_object(vm, arg.to_owned()) {
                Ok(s.as_str().as_bytes().to_vec())
            } else if let Ok(b) = ArgBytesLike::try_from_object(vm, arg.to_owned()) {
                Ok(b.borrow_buf().to_vec())
            } else {
                Err(vm.new_type_error("cadata should be a str or bytes".to_owned()))
            }
        }

        /// Helper: Update certificate statistics
        fn update_cert_stats(&self, stats: cert::CertStats) {
            *self.x509_cert_count.write() += stats.total_certs;
            *self.ca_cert_count.write() += stats.ca_certs;
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
            match protocol {
                PROTOCOL_TLS | PROTOCOL_TLS_CLIENT | PROTOCOL_TLS_SERVER | PROTOCOL_TLSv1_2
                | PROTOCOL_TLSv1_3 => {
                    // Valid protocols
                }
                PROTOCOL_TLSv1 | PROTOCOL_TLSv1_1 => {
                    return Err(vm.new_value_error(
                        "TLS 1.0 and 1.1 are not supported by rustls for security reasons",
                    ));
                }
                _ => {
                    return Err(vm.new_value_error(format!("invalid protocol version: {protocol}")));
                }
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

            // IMPORTANT: Create shared session cache BEFORE PySSLContext
            // Both client_session_cache and PythonClientSessionStore.session_cache
            // MUST point to the same HashMap to ensure Python-level and Rustls-level
            // sessions are synchronized
            let shared_session_cache = Arc::new(ParkingRwLock::new(HashMap::new()));
            let rustls_client_store = Arc::new(PythonClientSessionStore {
                inner: Arc::new(rustls::client::ClientSessionMemoryCache::new(
                    SSL_SESSION_CACHE_SIZE,
                )),
                session_cache: shared_session_cache.clone(),
            });

            Ok(PySSLContext {
                protocol,
                check_hostname: PyRwLock::new(protocol == PROTOCOL_TLS_CLIENT),
                verify_mode: PyRwLock::new(default_verify_mode),
                verify_flags: PyRwLock::new(default_verify_flags),
                client_config: PyRwLock::new(None),
                server_config: PyRwLock::new(None),
                root_certs: PyRwLock::new(RootCertStore::empty()),
                ca_certs_der: PyRwLock::new(Vec::new()),
                capath_certs_der: PyRwLock::new(Vec::new()),
                crls: PyRwLock::new(Vec::new()),
                cert_keys: PyRwLock::new(Vec::new()),
                options: PyRwLock::new(default_options),
                alpn_protocols: PyRwLock::new(Vec::new()),
                require_alpn_match: PyRwLock::new(false),
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
                rustls_session_store: rustls_client_store,
                rustls_server_session_store: rustls::server::ServerSessionMemoryCache::new(
                    SSL_SESSION_CACHE_SIZE,
                ),
                server_ticketer: rustls::crypto::aws_lc_rs::Ticketer::new()
                    .expect("Failed to create shared ticketer for TLS 1.2 session resumption"),
                accept_count: AtomicUsize::new(0),
                session_hits: AtomicUsize::new(0),
                selected_ciphers: PyRwLock::new(None),
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
        connection: PyMutex<Option<TlsConnection>>,
        // Handshake completed flag
        #[pytraverse(skip)]
        handshake_done: PyMutex<bool>,
        // Session was reused (for session resumption tracking)
        #[pytraverse(skip)]
        session_was_reused: PyMutex<bool>,
        // Owner (SSLSocket instance that owns this _SSLSocket)
        owner: PyRwLock<Option<PyObjectRef>>,
        // Session for resumption
        session: PyRwLock<Option<PyObjectRef>>,
        // Verified certificate chain (built during verification)
        #[allow(dead_code)]
        #[pytraverse(skip)]
        verified_chain: PyRwLock<Option<Vec<CertificateDer<'static>>>>,
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
        // Shutdown state for tracking close-notify exchange
        #[pytraverse(skip)]
        shutdown_state: PyMutex<ShutdownState>,
        // Pending TLS output buffer for non-blocking sockets
        // Stores unsent TLS bytes when sock_send() would block
        // This prevents data loss when write_tls() drains rustls' internal buffer
        // but the socket cannot accept all the data immediately
        #[pytraverse(skip)]
        pub(crate) pending_tls_output: PyMutex<Vec<u8>>,
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

    #[pyclass(with(Constructor), flags(BASETYPE))]
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
        fn create_session_after_handshake(&self, vm: &VirtualMachine) -> PyResult<()> {
            // Only create session for client-side connections
            if self.server_side {
                return Ok(());
            }

            // Check if session already exists
            let session_opt = self.session.read().clone();
            if let Some(ref s) = session_opt {
                if vm.is_none(s) {
                } else {
                    return Ok(());
                }
            }

            // Get server hostname
            let server_name = self.server_hostname.read().clone();

            // Try to get session data from context's session cache
            // IMPORTANT: Acquire and release locks quickly to avoid deadlock
            let context = self.context.read();
            let session_cache_arc = context.client_session_cache.clone();
            drop(context); // Release context lock ASAP

            let (session_id, creation_time, lifetime) = if let Some(ref name) = server_name {
                let key = name.as_bytes().to_vec();

                // Clone the data we need while holding the lock, then immediately release
                let session_data_opt = {
                    let cache_guard = session_cache_arc.read();
                    cache_guard.get(&key).cloned() // Clone Arc<PyMutex<SessionData>>
                }; // Lock released here

                if let Some(session_data_arc) = session_data_opt {
                    let data = session_data_arc.lock();
                    let result = (data.session_id.clone(), data.creation_time, data.lifetime);
                    drop(data); // Explicit unlock
                    result
                } else {
                    // Create new session ID if not in cache
                    let time = std::time::SystemTime::now();
                    (generate_session_id_from_metadata(name, &time), time, 7200)
                }
            } else {
                // No server name, use defaults
                let time = std::time::SystemTime::now();
                (vec![0; 16], time, 7200)
            };

            // Create a new SSLSession object with real metadata
            let session = PySSLSession {
                // Use dummy session data to indicate we have a ticket
                // TLS 1.2+ always uses session tickets/resumption
                session_data: vec![1], // Non-empty to indicate has_ticket=True
                session_id,
                creation_time,
                lifetime,
            };

            let py_session = session.into_pyobject(vm);

            *self.session.write() = Some(py_session);

            Ok(())
        }

        // Complete handshake and create session
        /// Track which CA certificate from capath was used to verify peer
        ///
        /// This simulates lazy loading behavior: capath certificates
        /// are only added to get_ca_certs() after they're actually used in a handshake.
        fn track_used_ca_from_capath(&self) -> Result<(), String> {
            // Extract capath_certs, releasing context lock quickly
            let capath_certs = {
                let context = self.context.read();
                let certs = context.capath_certs_der.read();
                if certs.is_empty() {
                    return Ok(());
                }
                certs.clone()
            };

            // Extract peer certificates, releasing connection lock quickly
            let top_cert_der = {
                let conn_guard = self.connection.lock();
                let conn = conn_guard.as_ref().ok_or("No connection")?;
                let peer_certs = conn.peer_certificates().ok_or("No peer certificates")?;
                if peer_certs.is_empty() {
                    return Ok(());
                }
                peer_certs
                    .iter()
                    .map(|c| c.as_ref().to_vec())
                    .next_back()
                    .expect("is_empty checked above")
            };

            // Get the top certificate in the chain (closest to root)
            // Note: Server usually doesn't send the root CA, so we check the last cert's issuer
            let (_, top_cert) = x509_parser::parse_x509_certificate(&top_cert_der)
                .map_err(|e| format!("Failed to parse top cert: {e}"))?;

            let top_issuer = top_cert.issuer();

            // Find matching CA in capath certs (skip unparseable certificates)
            let matching_ca = capath_certs.iter().find_map(|ca_der| {
                let (_, ca) = x509_parser::parse_x509_certificate(ca_der).ok()?;
                // Check if this CA is self-signed (root CA) and matches the issuer
                (ca.subject() == ca.issuer() && ca.subject() == top_issuer).then(|| ca_der.clone())
            });

            // Update ca_certs_der if we found a match
            if let Some(ca_der) = matching_ca {
                let context = self.context.read();
                let mut ca_certs_der = context.ca_certs_der.write();
                if !ca_certs_der.iter().any(|c| c == &ca_der) {
                    ca_certs_der.push(ca_der);
                }
            }

            Ok(())
        }

        fn complete_handshake(&self, vm: &VirtualMachine) -> PyResult<()> {
            *self.handshake_done.lock() = true;

            // Check if session was resumed - get value and release lock immediately
            let was_resumed = self
                .connection
                .lock()
                .as_ref()
                .map(|conn| conn.is_session_resumed())
                .unwrap_or(false);

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

            // Track CA certificate used during handshake (client-side only)
            // This simulates lazy loading behavior for capath certificates
            if !self.server_side {
                // Don't fail handshake if tracking fails
                let _ = self.track_used_ca_from_capath();
            }

            self.create_session_after_handshake(vm)?;
            Ok(())
        }

        // Internal implementation with timeout control
        pub(crate) fn sock_wait_for_io_impl(
            &self,
            kind: SelectKind,
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

            let timed_out = sock_select(&socket, kind, timeout)
                .map_err(|e| vm.new_os_error(format!("select failed: {e}")))?;

            Ok(timed_out)
        }

        // SNI (Server Name Indication) Helper Methods:
        // These methods support the server-side handshake SNI callback mechanism

        /// Check if this is the first read during handshake (for SNI callback)
        /// Returns true if we haven't processed ClientHello yet, regardless of SNI presence
        pub(crate) fn is_first_sni_read(&self) -> bool {
            self.client_hello_buffer.lock().is_none()
        }

        /// Check if SNI callback is configured
        pub(crate) fn has_sni_callback(&self) -> bool {
            // Nested read locks are safe
            self.context.read().sni_callback.read().is_some()
        }

        /// Save ClientHello data from PyObjectRef for potential connection recreation
        pub(crate) fn save_client_hello_from_bytes(&self, bytes_data: &[u8]) {
            *self.client_hello_buffer.lock() = Some(bytes_data.to_vec());
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
            let callback = self
                .context
                .read()
                .sni_callback
                .read()
                .clone()
                .ok_or_else(|| vm.new_value_error("SNI callback not set"))?;

            let ssl_sock = self.owner.read().clone().unwrap_or(vm.ctx.none());
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
                    vm.run_unraisable(type_error, None, result.clone());

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

            // Normal socket mode
            let socket_mod = vm.import("socket", 0)?;
            let socket_class = socket_mod.get_attr("socket", vm)?;

            // Call socket.socket.recv(self.sock, size, flags)
            let recv_method = socket_class.get_attr("recv", vm)?;
            recv_method.call((self.sock.clone(), vm.ctx.new_int(size)), vm)
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

            // Normal socket mode
            let socket_mod = vm.import("socket", 0)?;
            let socket_class = socket_mod.get_attr("socket", vm)?;

            // Call socket.socket.send(self.sock, data)
            let send_method = socket_class.get_attr("send", vm)?;
            send_method.call((self.sock.clone(), vm.ctx.new_bytes(data.to_vec())), vm)
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
            let is_non_blocking = socket_timeout.map(|t| t.is_zero()).unwrap_or(false);

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

                // Use sock_select directly with calculated timeout
                let py_socket: PyRef<PySocket> = self.sock.clone().try_into_value(vm)?;
                let socket = py_socket
                    .sock()
                    .map_err(|e| vm.new_os_error(format!("Failed to get socket: {e}")))?;
                let timed_out = sock_select(&socket, SelectKind::Write, timeout_to_use)
                    .map_err(|e| vm.new_os_error(format!("select failed: {e}")))?;

                if timed_out {
                    // Keep unsent data in pending buffer
                    *pending = pending[sent_total..].to_vec();
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
            let is_non_blocking = timeout.map(|t| t.is_zero()).unwrap_or(false);

            let mut sent_total = 0;
            while sent_total < buf.len() {
                let timed_out = self.sock_wait_for_io_impl(SelectKind::Write, vm)?;
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
                let timed_out = sock_select(&socket, SelectKind::Write, timeout)
                    .map_err(|e| vm.new_os_error(format!("select failed: {e}")))?;

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
                        // If sent == 0, socket wasn't ready despite select() saying so
                        // Continue loop to retry - this avoids infinite loops
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

        #[pymethod]
        fn __repr__(&self) -> String {
            "<SSLSocket>".to_string()
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
            drop(ctx);

            if let Some(ref curve_name) = ecdh_curve {
                match curve_name_to_kx_group(curve_name) {
                    Ok(groups) => Ok(Some(groups)),
                    Err(e) => Err(vm.new_value_error(format!("Failed to set ECDH curve: {e}"))),
                }
            } else {
                Ok(None)
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
            conn_guard: &mut Option<TlsConnection>,
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
            let root_store = ctx.root_certs.read();
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
                cert_chain: certs_clone,
                private_key: key_clone,
                root_store: if request_initial_cert {
                    Some(root_store.clone())
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

            drop(root_store);

            // Check if we have a cached ServerConfig
            let cached_config_arc = ctx.server_config.read().clone();
            drop(ctx);

            let config_arc = if let Some(cached) = cached_config_arc {
                // Don't use cache when SNI is enabled, because each connection needs
                // a fresh SniCertResolver with the correct Arc references
                if use_sni_resolver {
                    let config =
                        create_server_config(config_options).map_err(|e| vm.new_value_error(e))?;
                    Arc::new(config)
                } else {
                    cached
                }
            } else {
                let config =
                    create_server_config(config_options).map_err(|e| vm.new_value_error(e))?;
                let config_arc = Arc::new(config);

                // Cache the ServerConfig for future connections
                let ctx = self.context.read();
                *ctx.server_config.write() = Some(config_arc.clone());
                drop(ctx);

                config_arc
            };

            let conn = ServerConnection::new(config_arc).map_err(|e| {
                vm.new_value_error(format!("Failed to create server connection: {e}"))
            })?;

            *conn_guard = Some(TlsConnection::Server(conn));

            // If ClientHello buffer exists (from SNI callback), re-inject it
            if let Some(ref hello_data) = *self.client_hello_buffer.lock()
                && let Some(TlsConnection::Server(ref mut server)) = *conn_guard
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
                if let Some(new_ctx) = self.pending_context.write().take() {
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
                    let root_store_clone = ctx.root_certs.read().clone();
                    let ca_certs_der_clone = ctx.ca_certs_der.read().clone();

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

                    // Get session store before dropping ctx
                    let session_store = ctx.rustls_session_store.clone();

                    // Get CRLs for revocation checking
                    let crls_clone = ctx.crls.read().clone();

                    // Drop ctx early to avoid borrow conflicts
                    drop(ctx);

                    // Build client config using compat helper
                    let config_options = ClientConfigOptions {
                        protocol_settings,
                        root_store: if verify_mode != CERT_NONE {
                            Some(root_store_clone)
                        } else {
                            None
                        },
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
                        session_store: Some(session_store),
                        crls: crls_clone,
                    };

                    let config =
                        create_client_config(config_options).map_err(|e| vm.new_value_error(e))?;

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
                            core::net::IpAddr::V4(core::net::Ipv4Addr::new(127, 0, 0, 1)).into(),
                        )
                    };

                    let conn = ClientConnection::new(Arc::new(config), server_name.clone())
                        .map_err(|e| {
                            vm.new_value_error(format!("Failed to create client connection: {e}"))
                        })?;

                    *conn_guard = Some(TlsConnection::Client(conn));
                }
            }

            // Perform the actual handshake by exchanging data with the socket/BIO

            let conn = conn_guard.as_mut().expect("unreachable");
            let is_client = matches!(conn, TlsConnection::Client(_));
            let handshake_result = ssl_do_handshake(conn, self, vm);
            drop(conn_guard);

            if is_client {
                // CLIENT is simple - no SNI callback handling needed
                handshake_result.map_err(|e| e.into_py_err(vm))?;
                self.complete_handshake(vm)?;
                Ok(())
            } else {
                // Use OpenSSL-compatible handshake for server
                // Handle SNI callback restart
                match handshake_result {
                    Ok(()) => {
                        // Handshake completed successfully
                        self.complete_handshake(vm)?;
                        Ok(())
                    }
                    Err(SslError::SniCallbackRestart) => {
                        // SNI detected - need to call callback and recreate connection

                        // Get the SNI name that was extracted (may be None if client didn't send SNI)
                        let sni_name = self.get_extracted_sni_name();

                        // Now safe to call Python callback (no locks held)
                        self.invoke_sni_callback(sni_name.as_deref(), vm)?;

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
            // After unwrap()/shutdown(), read operations should fail with SSLError
            let shutdown_state = *self.shutdown_state.lock();
            if shutdown_state != ShutdownState::NotStarted {
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
            let mut buf = vec![0u8; len];
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
                    // EOF occurred in violation of protocol (unexpected closure)
                    Err(vm
                        .new_os_subtype_error(
                            PySSLEOFError::class(&vm.ctx).to_owned(),
                            None,
                            "EOF occurred in violation of protocol",
                        )
                        .upcast())
                }
                Err(crate::ssl::compat::SslError::ZeroReturn) => {
                    // Clean closure with close_notify - return empty data
                    return_data(vec![], &buffer, vm)
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
        fn pending(&self) -> PyResult<usize> {
            // Returns the number of already decrypted bytes available for read
            // This is critical for asyncore's readable() method which checks socket.pending() > 0
            let mut conn_guard = self.connection.lock();
            let conn = match conn_guard.as_mut() {
                Some(c) => c,
                None => return Ok(0), // No connection established yet
            };

            // Use rustls Reader's fill_buf() to check buffered plaintext
            // fill_buf() returns a reference to buffered data without consuming it
            // This matches OpenSSL's SSL_pending() behavior
            use std::io::BufRead;
            let mut reader = conn.reader();
            match reader.fill_buf() {
                Ok(buf) => Ok(buf.len()),
                Err(_) => {
                    // WouldBlock or other errors mean no data available
                    // Return 0 like OpenSSL does when buffer is empty
                    Ok(0)
                }
            }
        }

        #[pymethod]
        fn write(&self, data: ArgBytesLike, vm: &VirtualMachine) -> PyResult<usize> {
            let data_bytes = data.borrow_buf();
            let data_len = data_bytes.len();

            // return 0 immediately for empty write
            if data_len == 0 {
                return Ok(0);
            }

            // Ensure handshake is done - if not, complete it first
            // This matches OpenSSL behavior where SSL_write() auto-completes handshake
            if !*self.handshake_done.lock() {
                self.do_handshake(vm)?;
            }

            // Check if connection has been shut down
            // After unwrap()/shutdown(), write operations should fail with SSLError
            let shutdown_state = *self.shutdown_state.lock();
            if shutdown_state != ShutdownState::NotStarted {
                return Err(vm
                    .new_os_subtype_error(
                        PySSLError::class(&vm.ctx).to_owned(),
                        None,
                        "cannot write after shutdown",
                    )
                    .upcast());
            }

            {
                let mut conn_guard = self.connection.lock();
                let conn = conn_guard
                    .as_mut()
                    .ok_or_else(|| vm.new_value_error("Connection not established"))?;

                let is_bio = self.is_bio_mode();
                let data: &[u8] = data_bytes.as_ref();

                // CRITICAL: Flush any pending TLS data before writing new data
                // This ensures TLS 1.3 Finished message reaches server before application data
                // Without this, server may not be ready to process our data
                if !is_bio {
                    self.flush_pending_tls_output(vm, None)?;
                }

                // Write data in chunks to avoid filling the internal TLS buffer
                // rustls has a limited internal buffer, so we need to flush periodically
                const CHUNK_SIZE: usize = 16384; // 16KB chunks (typical TLS record size)
                let mut written = 0;

                while written < data.len() {
                    let chunk_end = core::cmp::min(written + CHUNK_SIZE, data.len());
                    let chunk = &data[written..chunk_end];

                    // Write chunk to TLS layer
                    {
                        let mut writer = conn.writer();
                        use std::io::Write;
                        writer
                            .write_all(chunk)
                            .map_err(|e| vm.new_os_error(format!("Write failed: {e}")))?;
                        // Flush to ensure data is converted to TLS records
                        writer
                            .flush()
                            .map_err(|e| vm.new_os_error(format!("Flush failed: {e}")))?;
                    }

                    written = chunk_end;

                    // Flush TLS data to socket after each chunk
                    if conn.wants_write() {
                        if is_bio {
                            self.write_pending_tls(conn, vm)?;
                        } else {
                            // Socket mode: flush all pending TLS data
                            // First, try to send any previously pending data
                            self.flush_pending_tls_output(vm, None)?;

                            while conn.wants_write() {
                                let mut buf = Vec::new();
                                conn.write_tls(&mut buf).map_err(|e| {
                                    vm.new_os_error(format!("TLS write failed: {e}"))
                                })?;

                                if !buf.is_empty() {
                                    // Try to send TLS data, saving unsent bytes to pending buffer
                                    self.send_tls_output(buf, vm)?;
                                }
                            }
                        }
                    }
                }
            }

            // Check for deferred certificate verification errors (TLS 1.3)
            // Must be checked AFTER write completes, as the error may be set during I/O
            self.check_deferred_cert_error(vm)?;

            Ok(data_len)
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
            let cipher_info = extract_cipher_info(&suite);

            // Note: returns a 3-tuple (name, protocol_version, bits)
            // The 'description' field is part of get_ciphers() output, not cipher()
            Some((
                cipher_info.name,
                cipher_info.protocol.to_string(),
                cipher_info.bits,
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
            self.owner.read().clone()
        }

        #[pygetset(setter)]
        fn set_owner(&self, owner: PyObjectRef, _vm: &VirtualMachine) -> PyResult<()> {
            *self.owner.write() = Some(owner);
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
        fn set_context(&self, value: PyRef<PySSLContext>, _vm: &VirtualMachine) -> PyResult<()> {
            // Update context reference immediately
            // SSL_set_SSL_CTX allows context changes at any time,
            // even after handshake completion
            *self.context.write() = value;

            // Clear pending context as we've applied the change
            *self.pending_context.write() = None;

            Ok(())
        }

        #[pygetset]
        fn server_hostname(&self) -> Option<String> {
            self.server_hostname.read().clone()
        }

        #[pygetset(setter)]
        fn set_server_hostname(
            &self,
            value: Option<PyStrRef>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            // Check if handshake is already done
            if *self.handshake_done.lock() {
                return Err(
                    vm.new_value_error("Cannot set server_hostname on socket after handshake")
                );
            }

            // Validate hostname
            if let Some(hostname_str) = &value {
                validate_hostname(hostname_str.as_str(), vm)?;
            }

            *self.server_hostname.write() = value.map(|s| s.as_str().to_string());
            Ok(())
        }

        #[pygetset]
        fn session(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            // Return the stored session object if any
            let sess = self.session.read().clone();
            if let Some(s) = sess {
                Ok(s)
            } else {
                Ok(vm.ctx.none())
            }
        }

        #[pygetset(setter)]
        fn set_session(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            // Validate that value is an SSLSession
            if !value.is(vm.ctx.types.none_type) {
                // Try to downcast to SSLSession to validate
                let _ = value
                    .downcast_ref::<PySSLSession>()
                    .ok_or_else(|| vm.new_type_error("Value is not a SSLSession."))?;
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
            *self.session.write() = if value.is(vm.ctx.types.none_type) {
                None
            } else {
                Some(value)
            };

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
        fn get_verified_chain(&self, vm: &VirtualMachine) -> PyResult<Option<PyListRef>> {
            // Get peer certificates (what peer sent during handshake)
            let conn_guard = self.connection.lock();
            let Some(ref conn) = *conn_guard else {
                return Ok(None);
            };

            let peer_certs = conn.peer_certificates();

            let Some(peer_certs_slice) = peer_certs else {
                return Ok(None);
            };

            // Build the verified chain using cert module
            let ctx_guard = self.context.read();
            let ca_certs_der = ctx_guard.ca_certs_der.read();

            let chain_der = cert::build_verified_chain(peer_certs_slice, &ca_certs_der);

            // Convert DER chain to Python list of Certificate objects
            let cert_list: Vec<PyObjectRef> = chain_der
                .into_iter()
                .map(|der_bytes| PySSLCertificate { der_bytes }.into_ref(&vm.ctx).into())
                .collect();

            Ok(Some(vm.ctx.new_list(cert_list)))
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
                    // All socket modes (blocking, timeout, non-blocking):
                    // Return immediately after sending our close_notify.
                    //
                    // This matches CPython/OpenSSL behavior where SSL_shutdown()
                    // returns after sending close_notify, allowing the app to
                    // close the socket without waiting for peer's close_notify.
                    //
                    // Waiting for peer's close_notify can cause deadlock with
                    // asyncore-based servers where both sides wait for the other's
                    // close_notify before closing the connection.

                    // Ensure all pending TLS data is sent before returning
                    // This prevents data loss when rustls drains its buffer
                    // but the socket couldn't accept all data immediately
                    drop(conn_guard);

                    // Respect socket timeout settings for flushing pending TLS data
                    match timeout {
                        Some(0.0) => {
                            // Non-blocking: best-effort flush, ignore errors
                            // to avoid deadlock with asyncore-based servers
                            let _ = self.flush_pending_tls_output(vm, None);
                        }
                        Some(_t) => {
                            // Timeout mode: use flush with socket's timeout
                            // Errors (including timeout) are propagated to caller
                            self.flush_pending_tls_output(vm, None)?;
                        }
                        None => {
                            // Blocking mode: wait until all pending data is sent
                            self.blocking_flush_all_pending(vm)?;
                        }
                    }

                    *self.shutdown_state.lock() = ShutdownState::Completed;
                    *self.connection.lock() = None;
                    return Ok(self.sock.clone());
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
        fn write_pending_tls(&self, conn: &mut TlsConnection, vm: &VirtualMachine) -> PyResult<()> {
            // First, flush any previously pending TLS output
            // Must succeed before sending new data to maintain order
            self.flush_pending_tls_output(vm, None)?;

            loop {
                if !conn.wants_write() {
                    break;
                }

                let mut buf = vec![0u8; SSL3_RT_MAX_PLAIN_LENGTH];
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
            conn: &mut TlsConnection,
            vm: &VirtualMachine,
        ) -> PyResult<bool> {
            // Try to read incoming data
            match self.sock_recv(SSL3_RT_MAX_PLAIN_LENGTH, vm) {
                Ok(bytes_obj) => {
                    let bytes = ArgBytesLike::try_from_object(vm, bytes_obj)?;
                    let data = bytes.borrow_buf();

                    if data.is_empty() {
                        // Empty read could mean EOF or just "no data yet" in BIO mode
                        if let Some(ref bio) = self.incoming_bio {
                            // BIO mode: check if EOF was signaled via write_eof()
                            let bio_obj: PyObjectRef = bio.clone().into();
                            let eof_attr = bio_obj.get_attr("eof", vm)?;
                            let is_eof = eof_attr.try_to_bool(vm)?;
                            if !is_eof {
                                // No EOF signaled, just no data available yet
                                return Ok(false);
                            }
                        }
                        // Socket mode or BIO with EOF: peer closed connection
                        // This is "ragged EOF" - peer closed without close_notify
                        return Ok(true);
                    }

                    // Feed data to TLS connection
                    let data_slice: &[u8] = data.as_ref();
                    let mut cursor = std::io::Cursor::new(data_slice);
                    let _ = conn.read_tls(&mut cursor);

                    // Process packets
                    let _ = conn.process_new_packets();
                    Ok(false)
                }
                Err(e) => {
                    // BlockingIOError means no data yet
                    if is_blocking_io_error(&e, vm) {
                        return Ok(false);
                    }
                    // Connection reset, EOF, or other error means peer closed
                    // ECONNRESET, EPIPE, broken pipe, etc.
                    Ok(true)
                }
            }
        }

        // Helper: Check if peer has sent close_notify
        fn check_peer_closed(
            &self,
            conn: &mut TlsConnection,
            vm: &VirtualMachine,
        ) -> PyResult<bool> {
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

            // Extract cipher information using unified helper
            let cipher_info = extract_cipher_info(&suite);

            // Return as list with single tuple (name, version, bits)
            let tuple = vm.ctx.new_tuple(vec![
                vm.ctx.new_str(cipher_info.name).into(),
                vm.ctx.new_str(cipher_info.protocol).into(),
                vm.ctx.new_int(cipher_info.bits).into(),
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
                    TlsConnection::Client(_) => {
                        return Err(vm.new_value_error(
                            "Post-handshake authentication requires server socket",
                        ));
                    }
                    TlsConnection::Server(server) => server.protocol_version(),
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
            cb_type: OptionalArg<PyStrRef>,
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
                    return Err(vm.new_exception_msg(
                        vm.ctx.exceptions.buffer_error.to_owned(),
                        "non-contiguous buffer is not supported".to_owned(),
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
        fn write_eof(&self, _vm: &VirtualMachine) -> PyResult<()> {
            *self.eof.write() = true;
            Ok(())
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
            Ok(PyMemoryBIO {
                buffer: PyMutex::new(Vec::new()),
                eof: PyRwLock::new(false),
            })
        }
    }

    // SSLSession - represents a cached SSL session
    // NOTE: This is an EMULATION - actual session data is managed by Rustls internally
    #[pyattr]
    #[pyclass(name = "SSLSession", module = "ssl")]
    #[derive(Debug, PyPayload)]
    struct PySSLSession {
        // Session data - serialized rustls session (EMULATED - kept empty)
        session_data: Vec<u8>,
        // Session ID - synthetic ID generated from metadata (NOT actual TLS session ID)
        #[allow(dead_code)]
        session_id: Vec<u8>,
        // Session metadata
        creation_time: std::time::SystemTime,
        // Lifetime in seconds (default 7200 = 2 hours)
        lifetime: u64,
    }

    #[pyclass(flags(BASETYPE))]
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
            // Return session ID (hash of session data for uniqueness)
            use core::hash::{Hash, Hasher};
            use std::collections::hash_map::DefaultHasher;

            let mut hasher = DefaultHasher::new();
            self.session_data.hash(&mut hasher);
            let hash = hasher.finish();

            // Convert hash to bytes
            vm.ctx.new_bytes(hash.to_be_bytes().to_vec())
        }

        #[pygetset]
        fn has_ticket(&self) -> bool {
            // For rustls, if we have session data, we have a ticket
            !self.session_data.is_empty()
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
        txt: PyStrRef,
        #[pyarg(named, optional)]
        name: OptionalArg<bool>,
    }

    #[pyfunction]
    fn txt2obj(args: Txt2ObjArgs, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let txt = args.txt.as_str();
        let name = args.name.unwrap_or(false);

        // If name=False (default), only accept OID strings
        // If name=True, accept both names and OID strings
        let entry = if txt
            .chars()
            .next()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false)
        {
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
                vm.ctx.new_str(entry.oid_string()),
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
                vm.ctx.new_str(entry.oid_string()),
            ))
            .into())
    }

    #[pyfunction]
    fn get_default_verify_paths(vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        // Return default certificate paths as a tuple
        // Lib/ssl.py expects: (openssl_cafile_env, openssl_cafile, openssl_capath_env, openssl_capath)
        // parts[0] = environment variable name for cafile
        // parts[1] = default cafile path
        // parts[2] = environment variable name for capath
        // parts[3] = default capath path

        // Common default paths for different platforms
        // These match the first candidates that rustls-native-certs/openssl-probe checks
        #[cfg(target_os = "macos")]
        let (default_cafile, default_capath) = {
            // macOS primarily uses Keychain API, but provides fallback paths
            // for compatibility and when Keychain access fails
            (Some("/etc/ssl/cert.pem"), Some("/etc/ssl/certs"))
        };

        #[cfg(target_os = "linux")]
        let (default_cafile, default_capath) = {
            // Linux: matches openssl-probe's first candidate (/etc/ssl/cert.pem)
            // openssl-probe checks multiple locations at runtime, but we return
            // OpenSSL's compile-time default
            (Some("/etc/ssl/cert.pem"), Some("/etc/ssl/certs"))
        };

        #[cfg(windows)]
        let (default_cafile, default_capath) = {
            // Windows uses certificate store, not file paths
            // Return empty strings to avoid None being passed to os.path.isfile()
            (Some(""), Some(""))
        };

        #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
        let (default_cafile, default_capath): (Option<&str>, Option<&str>) = (None, None);

        let tuple = vm.ctx.new_tuple(vec![
            vm.ctx.new_str("SSL_CERT_FILE").into(), // openssl_cafile_env
            default_cafile
                .map(|s| vm.ctx.new_str(s).into())
                .unwrap_or_else(|| vm.ctx.none()), // openssl_cafile
            vm.ctx.new_str("SSL_CERT_DIR").into(),  // openssl_capath_env
            default_capath
                .map(|s| vm.ctx.new_str(s).into())
                .unwrap_or_else(|| vm.ctx.none()), // openssl_capath
        ]);
        Ok(tuple.into())
    }

    #[pyfunction]
    fn RAND_status() -> i32 {
        1 // Always have good randomness with aws-lc-rs
    }

    #[pyfunction]
    fn RAND_add(_string: PyObjectRef, _entropy: f64) {
        // No-op: aws-lc-rs handles its own entropy
        // Accept any type (str, bytes, bytearray)
    }

    #[pyfunction]
    fn RAND_bytes(n: i64, vm: &VirtualMachine) -> PyResult<PyBytesRef> {
        use aws_lc_rs::rand::{SecureRandom, SystemRandom};

        // Validate n is not negative
        if n < 0 {
            return Err(vm.new_value_error("num must be positive"));
        }

        let n_usize = n as usize;
        let rng = SystemRandom::new();
        let mut buf = vec![0u8; n_usize];
        rng.fill(&mut buf)
            .map_err(|_| vm.new_os_error("Failed to generate random bytes"))?;
        Ok(PyBytesRef::from(vm.ctx.new_bytes(buf)))
    }

    #[pyfunction]
    fn RAND_pseudo_bytes(n: i64, vm: &VirtualMachine) -> PyResult<(PyBytesRef, bool)> {
        // In rustls/aws-lc-rs, all random bytes are cryptographically strong
        let bytes = RAND_bytes(n, vm)?;
        Ok((bytes, true))
    }

    /// Test helper to decode a certificate from a file path
    ///
    /// This is a simplified wrapper around cert_der_to_dict_helper that handles
    /// file reading and PEM/DER auto-detection. Used by test suite.
    #[pyfunction]
    fn _test_decode_cert(path: PyStrRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        // Read certificate file
        let cert_data = std::fs::read(path.as_str()).map_err(|e| {
            vm.new_os_error(format!(
                "Failed to read certificate file {}: {}",
                path.as_str(),
                e
            ))
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
    fn PEM_cert_to_DER_cert(pem_cert: PyStrRef, vm: &VirtualMachine) -> PyResult<PyBytesRef> {
        let pem_str = pem_cert.as_str();

        // Parse PEM format
        let mut cursor = std::io::Cursor::new(pem_str.as_bytes());
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
    fn enum_certificates(store_name: PyStrRef, vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
        use schannel::{RawPointer, cert_context::ValidUses, cert_store::CertStore};
        use windows_sys::Win32::Security::Cryptography;

        // Try both Current User and Local Machine stores
        let open_fns = [CertStore::open_current_user, CertStore::open_local_machine];
        let stores = open_fns
            .iter()
            .filter_map(|open| open(store_name.as_str()).ok())
            .collect::<Vec<_>>();

        // If no stores could be opened, raise OSError
        if stores.is_empty() {
            return Err(vm.new_os_error(format!(
                "failed to open certificate store {:?}",
                store_name.as_str()
            )));
        }

        let certs = stores.iter().flat_map(|s| s.certs()).map(|c| {
            let cert = vm.ctx.new_bytes(c.to_der().to_owned());
            let enc_type = unsafe {
                let ptr = c.as_ptr() as *const Cryptography::CERT_CONTEXT;
                (*ptr).dwCertEncodingType
            };
            let enc_type = match enc_type {
                Cryptography::X509_ASN_ENCODING => vm.new_pyobj("x509_asn"),
                Cryptography::PKCS_7_ASN_ENCODING => vm.new_pyobj("pkcs_7_asn"),
                other => vm.new_pyobj(other),
            };
            let usage: PyObjectRef = match c.valid_uses() {
                Ok(ValidUses::All) => vm.ctx.new_bool(true).into(),
                Ok(ValidUses::Oids(oids)) => {
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
    fn enum_crls(store_name: PyStrRef, vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
        use windows_sys::Win32::Security::Cryptography::{
            CRL_CONTEXT, CertCloseStore, CertEnumCRLsInStore, CertOpenSystemStoreW,
            X509_ASN_ENCODING,
        };

        let store_name_wide: Vec<u16> = store_name
            .as_str()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        // Open system store
        let store = unsafe { CertOpenSystemStoreW(0, store_name_wide.as_ptr()) };

        if store.is_null() {
            return Err(vm.new_os_error(format!(
                "failed to open certificate store {:?}",
                store_name.as_str()
            )));
        }

        let mut result = Vec::new();

        let mut crl_context: *const CRL_CONTEXT = core::ptr::null();
        loop {
            crl_context = unsafe { CertEnumCRLsInStore(store, crl_context) };
            if crl_context.is_null() {
                break;
            }

            let crl = unsafe { &*crl_context };
            let crl_bytes =
                unsafe { std::slice::from_raw_parts(crl.pbCrlEncoded, crl.cbCrlEncoded as usize) };

            let enc_type = if crl.dwCertEncodingType == X509_ASN_ENCODING {
                vm.new_pyobj("x509_asn")
            } else {
                vm.new_pyobj(crl.dwCertEncodingType)
            };

            result.push(
                vm.new_tuple((vm.ctx.new_bytes(crl_bytes.to_vec()), enc_type))
                    .into(),
            );
        }

        unsafe { CertCloseStore(store, 0) };

        Ok(result)
    }

    // Certificate type for SSL module (pure Rust implementation)
    #[pyattr]
    #[pyclass(module = "_ssl", name = "Certificate")]
    #[derive(Debug, PyPayload)]
    pub struct PySSLCertificate {
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

    #[pyclass(with(Comparable, Hashable, Representable))]
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
            use core::hash::{Hash, Hasher};
            use std::collections::hash_map::DefaultHasher;

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
