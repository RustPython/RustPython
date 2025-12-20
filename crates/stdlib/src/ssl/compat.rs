// spell-checker: ignore webpki ssleof sslerror akid certsign sslerr aesgcm

// OpenSSL compatibility layer for rustls
//
// This module provides OpenSSL-like abstractions over rustls APIs,
// making the code more readable and maintainable. Each function is named
// after its OpenSSL equivalent (e.g., ssl_do_handshake corresponds to SSL_do_handshake).

// SSL error code data tables (shared with OpenSSL backend for compatibility)
// These map OpenSSL error codes to human-readable strings
#[path = "../openssl/ssl_data_31.rs"]
mod ssl_data;

use crate::socket::{SelectKind, timeout_error_msg};
use crate::vm::VirtualMachine;
use parking_lot::RwLock as ParkingRwLock;
use rustls::RootCertStore;
use rustls::client::ClientConfig;
use rustls::client::ClientConnection;
use rustls::crypto::SupportedKxGroup;
use rustls::pki_types::{CertificateDer, CertificateRevocationListDer, PrivateKeyDer};
use rustls::server::ResolvesServerCert;
use rustls::server::ServerConfig;
use rustls::server::ServerConnection;
use rustls::sign::CertifiedKey;
use rustpython_vm::builtins::{PyBaseException, PyBaseExceptionRef};
use rustpython_vm::convert::IntoPyException;
use rustpython_vm::function::ArgBytesLike;
use rustpython_vm::{AsObject, Py, PyObjectRef, PyPayload, PyResult, TryFromObject};
use std::io::Read;
use std::sync::{Arc, Once};

// Import PySSLSocket from parent module
use super::_ssl::PySSLSocket;

// Import error types and helper functions from error module
use super::error::{
    PySSLCertVerificationError, PySSLError, create_ssl_eof_error, create_ssl_want_read_error,
    create_ssl_want_write_error, create_ssl_zero_return_error,
};

// SSL Verification Flags
/// VERIFY_X509_STRICT flag for RFC 5280 strict compliance
/// When set, performs additional validation including AKI extension checks
pub const VERIFY_X509_STRICT: i32 = 0x20;

/// VERIFY_X509_PARTIAL_CHAIN flag for partial chain validation
/// When set, accept certificates if any certificate in the chain is in the trust store
/// (not just root CAs). This matches OpenSSL's X509_V_FLAG_PARTIAL_CHAIN behavior.
pub const VERIFY_X509_PARTIAL_CHAIN: i32 = 0x80000;

// CryptoProvider Initialization:

/// Ensure the default CryptoProvider is installed (thread-safe, runs once)
///
/// This is necessary because rustls 0.23+ requires a process-level CryptoProvider
/// to be installed before using default_provider(). We use Once to ensure this
/// happens exactly once, even if called from multiple threads.
static INIT_PROVIDER: Once = Once::new();

fn ensure_default_provider() {
    INIT_PROVIDER.call_once(|| {
        let _ = rustls::crypto::CryptoProvider::install_default(
            rustls::crypto::aws_lc_rs::default_provider(),
        );
    });
}

// OpenSSL Constants:

// OpenSSL TLS record maximum plaintext size (ssl/ssl_local.h)
// #define SSL3_RT_MAX_PLAIN_LENGTH 16384
const SSL3_RT_MAX_PLAIN_LENGTH: usize = 16384;

// OpenSSL error library codes (include/openssl/err.h)
// #define ERR_LIB_SSL 20
const ERR_LIB_SSL: i32 = 20;

// OpenSSL SSL error reason codes (include/openssl/sslerr.h)
// #define SSL_R_NO_SHARED_CIPHER 193
const SSL_R_NO_SHARED_CIPHER: i32 = 193;

// OpenSSL X509 verification flags (include/openssl/x509_vfy.h)
// #define X509_V_FLAG_CRL_CHECK 4
const X509_V_FLAG_CRL_CHECK: i32 = 4;

// X509 Certificate Verification Error Codes (OpenSSL Compatible):
//
// These constants match OpenSSL's X509_V_ERR_* values for certificate
// verification. They are used to map rustls certificate errors to OpenSSL
// error codes for compatibility.

pub use x509::{
    X509_V_ERR_CERT_HAS_EXPIRED, X509_V_ERR_CERT_NOT_YET_VALID, X509_V_ERR_CERT_REVOKED,
    X509_V_ERR_HOSTNAME_MISMATCH, X509_V_ERR_INVALID_PURPOSE, X509_V_ERR_IP_ADDRESS_MISMATCH,
    X509_V_ERR_UNABLE_TO_GET_CRL, X509_V_ERR_UNABLE_TO_GET_ISSUER_CERT_LOCALLY,
    X509_V_ERR_UNSPECIFIED,
};

#[allow(dead_code)]
mod x509 {
    pub const X509_V_OK: i32 = 0;
    pub const X509_V_ERR_UNSPECIFIED: i32 = 1;
    pub const X509_V_ERR_UNABLE_TO_GET_ISSUER_CERT: i32 = 2;
    pub const X509_V_ERR_UNABLE_TO_GET_CRL: i32 = 3;
    pub const X509_V_ERR_UNABLE_TO_DECRYPT_CERT_SIGNATURE: i32 = 4;
    pub const X509_V_ERR_UNABLE_TO_DECRYPT_CRL_SIGNATURE: i32 = 5;
    pub const X509_V_ERR_UNABLE_TO_DECODE_ISSUER_PUBLIC_KEY: i32 = 6;
    pub const X509_V_ERR_CERT_SIGNATURE_FAILURE: i32 = 7;
    pub const X509_V_ERR_CRL_SIGNATURE_FAILURE: i32 = 8;
    pub const X509_V_ERR_CERT_NOT_YET_VALID: i32 = 9;
    pub const X509_V_ERR_CERT_HAS_EXPIRED: i32 = 10;
    pub const X509_V_ERR_CRL_NOT_YET_VALID: i32 = 11;
    pub const X509_V_ERR_CRL_HAS_EXPIRED: i32 = 12;
    pub const X509_V_ERR_ERROR_IN_CERT_NOT_BEFORE_FIELD: i32 = 13;
    pub const X509_V_ERR_ERROR_IN_CERT_NOT_AFTER_FIELD: i32 = 14;
    pub const X509_V_ERR_ERROR_IN_CRL_LAST_UPDATE_FIELD: i32 = 15;
    pub const X509_V_ERR_ERROR_IN_CRL_NEXT_UPDATE_FIELD: i32 = 16;
    pub const X509_V_ERR_OUT_OF_MEM: i32 = 17;
    pub const X509_V_ERR_DEPTH_ZERO_SELF_SIGNED_CERT: i32 = 18;
    pub const X509_V_ERR_SELF_SIGNED_CERT_IN_CHAIN: i32 = 19;
    pub const X509_V_ERR_UNABLE_TO_GET_ISSUER_CERT_LOCALLY: i32 = 20;
    pub const X509_V_ERR_UNABLE_TO_VERIFY_LEAF_SIGNATURE: i32 = 21;
    pub const X509_V_ERR_CERT_CHAIN_TOO_LONG: i32 = 22;
    pub const X509_V_ERR_CERT_REVOKED: i32 = 23;
    pub const X509_V_ERR_INVALID_CA: i32 = 24;
    pub const X509_V_ERR_PATH_LENGTH_EXCEEDED: i32 = 25;
    pub const X509_V_ERR_INVALID_PURPOSE: i32 = 26;
    pub const X509_V_ERR_CERT_UNTRUSTED: i32 = 27;
    pub const X509_V_ERR_CERT_REJECTED: i32 = 28;
    pub const X509_V_ERR_SUBJECT_ISSUER_MISMATCH: i32 = 29;
    pub const X509_V_ERR_AKID_SKID_MISMATCH: i32 = 30;
    pub const X509_V_ERR_AKID_ISSUER_SERIAL_MISMATCH: i32 = 31;
    pub const X509_V_ERR_KEYUSAGE_NO_CERTSIGN: i32 = 32;
    pub const X509_V_ERR_UNABLE_TO_GET_CRL_ISSUER: i32 = 33;
    pub const X509_V_ERR_UNHANDLED_CRITICAL_EXTENSION: i32 = 34;
    pub const X509_V_ERR_KEYUSAGE_NO_CRL_SIGN: i32 = 35;
    pub const X509_V_ERR_UNHANDLED_CRITICAL_CRL_EXTENSION: i32 = 36;
    pub const X509_V_ERR_INVALID_NON_CA: i32 = 37;
    pub const X509_V_ERR_PROXY_PATH_LENGTH_EXCEEDED: i32 = 38;
    pub const X509_V_ERR_KEYUSAGE_NO_DIGITAL_SIGNATURE: i32 = 39;
    pub const X509_V_ERR_PROXY_CERTIFICATES_NOT_ALLOWED: i32 = 40;
    pub const X509_V_ERR_INVALID_EXTENSION: i32 = 41;
    pub const X509_V_ERR_INVALID_POLICY_EXTENSION: i32 = 42;
    pub const X509_V_ERR_NO_EXPLICIT_POLICY: i32 = 43;
    pub const X509_V_ERR_DIFFERENT_CRL_SCOPE: i32 = 44;
    pub const X509_V_ERR_UNSUPPORTED_EXTENSION_FEATURE: i32 = 45;
    pub const X509_V_ERR_UNNESTED_RESOURCE: i32 = 46;
    pub const X509_V_ERR_PERMITTED_VIOLATION: i32 = 47;
    pub const X509_V_ERR_EXCLUDED_VIOLATION: i32 = 48;
    pub const X509_V_ERR_SUBTREE_MINMAX: i32 = 49;
    pub const X509_V_ERR_APPLICATION_VERIFICATION: i32 = 50;
    pub const X509_V_ERR_UNSUPPORTED_CONSTRAINT_TYPE: i32 = 51;
    pub const X509_V_ERR_UNSUPPORTED_CONSTRAINT_SYNTAX: i32 = 52;
    pub const X509_V_ERR_UNSUPPORTED_NAME_SYNTAX: i32 = 53;
    pub const X509_V_ERR_CRL_PATH_VALIDATION_ERROR: i32 = 54;
    pub const X509_V_ERR_HOSTNAME_MISMATCH: i32 = 62;
    pub const X509_V_ERR_EMAIL_MISMATCH: i32 = 63;
    pub const X509_V_ERR_IP_ADDRESS_MISMATCH: i32 = 64;
}

// Certificate Error Conversion Functions:

/// Convert rustls CertificateError to X509 verification code and message
///
/// Maps rustls certificate errors to OpenSSL X509_V_ERR_* codes for compatibility.
/// Returns (verify_code, verify_message) tuple.
fn rustls_cert_error_to_verify_info(cert_err: &rustls::CertificateError) -> (i32, &'static str) {
    use rustls::CertificateError;

    match cert_err {
        CertificateError::UnknownIssuer => (
            X509_V_ERR_UNABLE_TO_GET_ISSUER_CERT_LOCALLY,
            "unable to get local issuer certificate",
        ),
        CertificateError::Expired => (X509_V_ERR_CERT_HAS_EXPIRED, "certificate has expired"),
        CertificateError::NotValidYet => (
            X509_V_ERR_CERT_NOT_YET_VALID,
            "certificate is not yet valid",
        ),
        CertificateError::Revoked => (X509_V_ERR_CERT_REVOKED, "certificate revoked"),
        CertificateError::UnknownRevocationStatus => (
            X509_V_ERR_UNABLE_TO_GET_CRL,
            "unable to get certificate CRL",
        ),
        CertificateError::InvalidPurpose => (
            X509_V_ERR_INVALID_PURPOSE,
            "unsupported certificate purpose",
        ),
        CertificateError::Other(other_err) => {
            // Check if this is a hostname mismatch error from our verify_hostname function
            let err_msg = format!("{other_err:?}");
            if err_msg.contains("Hostname mismatch") || err_msg.contains("not valid for") {
                (
                    X509_V_ERR_HOSTNAME_MISMATCH,
                    "Hostname mismatch, certificate is not valid for",
                )
            } else if err_msg.contains("IP address mismatch") {
                (
                    X509_V_ERR_IP_ADDRESS_MISMATCH,
                    "IP address mismatch, certificate is not valid for",
                )
            } else {
                (X509_V_ERR_UNSPECIFIED, "certificate verification failed")
            }
        }
        _ => (X509_V_ERR_UNSPECIFIED, "certificate verification failed"),
    }
}

/// Create SSLCertVerificationError with proper attributes
///
/// Matches CPython's _ssl.c fill_and_set_sslerror() behavior.
/// This function creates a Python SSLCertVerificationError exception with verify_code
/// and verify_message attributes set appropriately for the given rustls certificate error.
///
/// # Note
/// If attribute setting fails (extremely rare), returns the exception without attributes
pub(super) fn create_ssl_cert_verification_error(
    vm: &VirtualMachine,
    cert_err: &rustls::CertificateError,
) -> PyResult<PyBaseExceptionRef> {
    let (verify_code, verify_message) = rustls_cert_error_to_verify_info(cert_err);

    let msg =
        format!("[SSL: CERTIFICATE_VERIFY_FAILED] certificate verify failed: {verify_message}",);

    let exc = vm.new_os_subtype_error(
        PySSLCertVerificationError::class(&vm.ctx).to_owned(),
        None,
        msg,
    );

    // Set verify_code and verify_message attributes
    // Ignore errors as they're extremely rare (e.g., out of memory)
    exc.as_object().set_attr(
        "verify_code",
        vm.ctx.new_int(verify_code).as_object().to_owned(),
        vm,
    )?;
    exc.as_object().set_attr(
        "verify_message",
        vm.ctx.new_str(verify_message).as_object().to_owned(),
        vm,
    )?;

    exc.as_object()
        .set_attr("library", vm.ctx.new_str("SSL").as_object().to_owned(), vm)?;
    exc.as_object().set_attr(
        "reason",
        vm.ctx
            .new_str("CERTIFICATE_VERIFY_FAILED")
            .as_object()
            .to_owned(),
        vm,
    )?;

    Ok(exc.upcast())
}

/// Unified TLS connection type (client or server)
#[derive(Debug)]
pub(super) enum TlsConnection {
    Client(ClientConnection),
    Server(ServerConnection),
}

impl TlsConnection {
    /// Check if handshake is in progress
    pub fn is_handshaking(&self) -> bool {
        match self {
            TlsConnection::Client(conn) => conn.is_handshaking(),
            TlsConnection::Server(conn) => conn.is_handshaking(),
        }
    }

    /// Check if connection wants to read data
    pub fn wants_read(&self) -> bool {
        match self {
            TlsConnection::Client(conn) => conn.wants_read(),
            TlsConnection::Server(conn) => conn.wants_read(),
        }
    }

    /// Check if connection wants to write data
    pub fn wants_write(&self) -> bool {
        match self {
            TlsConnection::Client(conn) => conn.wants_write(),
            TlsConnection::Server(conn) => conn.wants_write(),
        }
    }

    /// Read TLS data from socket
    pub fn read_tls(&mut self, reader: &mut dyn std::io::Read) -> std::io::Result<usize> {
        match self {
            TlsConnection::Client(conn) => conn.read_tls(reader),
            TlsConnection::Server(conn) => conn.read_tls(reader),
        }
    }

    /// Write TLS data to socket
    pub fn write_tls(&mut self, writer: &mut dyn std::io::Write) -> std::io::Result<usize> {
        match self {
            TlsConnection::Client(conn) => conn.write_tls(writer),
            TlsConnection::Server(conn) => conn.write_tls(writer),
        }
    }

    /// Process new TLS packets
    pub fn process_new_packets(&mut self) -> Result<rustls::IoState, rustls::Error> {
        match self {
            TlsConnection::Client(conn) => conn.process_new_packets(),
            TlsConnection::Server(conn) => conn.process_new_packets(),
        }
    }

    /// Get reader for plaintext data (rustls native type)
    pub fn reader(&mut self) -> rustls::Reader<'_> {
        match self {
            TlsConnection::Client(conn) => conn.reader(),
            TlsConnection::Server(conn) => conn.reader(),
        }
    }

    /// Get writer for plaintext data (rustls native type)
    pub fn writer(&mut self) -> rustls::Writer<'_> {
        match self {
            TlsConnection::Client(conn) => conn.writer(),
            TlsConnection::Server(conn) => conn.writer(),
        }
    }

    /// Check if session was resumed
    pub fn is_session_resumed(&self) -> bool {
        use rustls::HandshakeKind;
        match self {
            TlsConnection::Client(conn) => {
                matches!(conn.handshake_kind(), Some(HandshakeKind::Resumed))
            }
            TlsConnection::Server(conn) => {
                matches!(conn.handshake_kind(), Some(HandshakeKind::Resumed))
            }
        }
    }

    /// Send close_notify alert
    pub fn send_close_notify(&mut self) {
        match self {
            TlsConnection::Client(conn) => conn.send_close_notify(),
            TlsConnection::Server(conn) => conn.send_close_notify(),
        }
    }

    /// Get negotiated ALPN protocol
    pub fn alpn_protocol(&self) -> Option<&[u8]> {
        match self {
            TlsConnection::Client(conn) => conn.alpn_protocol(),
            TlsConnection::Server(conn) => conn.alpn_protocol(),
        }
    }

    /// Get negotiated cipher suite
    pub fn negotiated_cipher_suite(&self) -> Option<rustls::SupportedCipherSuite> {
        match self {
            TlsConnection::Client(conn) => conn.negotiated_cipher_suite(),
            TlsConnection::Server(conn) => conn.negotiated_cipher_suite(),
        }
    }

    /// Get peer certificates
    pub fn peer_certificates(&self) -> Option<&[rustls::pki_types::CertificateDer<'static>]> {
        match self {
            TlsConnection::Client(conn) => conn.peer_certificates(),
            TlsConnection::Server(conn) => conn.peer_certificates(),
        }
    }
}

/// Error types matching OpenSSL error codes
#[derive(Debug)]
pub(super) enum SslError {
    /// SSL_ERROR_WANT_READ
    WantRead,
    /// SSL_ERROR_WANT_WRITE
    WantWrite,
    /// SSL_ERROR_SYSCALL
    Syscall(String),
    /// SSL_ERROR_SSL
    Ssl(String),
    /// SSL_ERROR_ZERO_RETURN (clean closure with close_notify)
    ZeroReturn,
    /// Unexpected EOF without close_notify (protocol violation)
    Eof,
    /// Certificate verification error
    CertVerification(rustls::CertificateError),
    /// I/O error
    Io(std::io::Error),
    /// Timeout error (socket.timeout)
    Timeout(String),
    /// SNI callback triggered - need to restart handshake
    SniCallbackRestart,
    /// Python exception (pass through directly)
    Py(PyBaseExceptionRef),
    /// TLS alert received with OpenSSL-compatible error code
    AlertReceived { lib: i32, reason: i32 },
    /// NO_SHARED_CIPHER error (OpenSSL SSL_R_NO_SHARED_CIPHER)
    NoCipherSuites,
}

impl SslError {
    /// Convert TLS alert code to OpenSSL error reason code
    /// OpenSSL uses reason = 1000 + alert_code for TLS alerts
    fn alert_to_openssl_reason(alert: rustls::AlertDescription) -> i32 {
        // AlertDescription can be converted to u8 via as u8 cast
        1000 + (u8::from(alert) as i32)
    }

    /// Convert rustls error to SslError
    pub fn from_rustls(err: rustls::Error) -> Self {
        match err {
            rustls::Error::InvalidCertificate(cert_err) => SslError::CertVerification(cert_err),
            rustls::Error::AlertReceived(alert_desc) => {
                // Map TLS alerts to OpenSSL-compatible error codes
                // lib = 20 (ERR_LIB_SSL), reason = 1000 + alert_code
                match alert_desc {
                    rustls::AlertDescription::CloseNotify => {
                        // Special case: close_notify is handled as ZeroReturn
                        SslError::ZeroReturn
                    }
                    _ => {
                        // All other alerts: convert to OpenSSL error code
                        // This includes InternalError (80 -> reason 1080)
                        SslError::AlertReceived {
                            lib: ERR_LIB_SSL,
                            reason: Self::alert_to_openssl_reason(alert_desc),
                        }
                    }
                }
            }
            // OpenSSL 3.0 changed transport EOF from SSL_ERROR_SYSCALL with
            // zero return value to SSL_ERROR_SSL with SSL_R_UNEXPECTED_EOF_WHILE_READING.
            // In rustls, these cases correspond to unexpected connection closure:
            rustls::Error::InvalidMessage(_) => {
                // UnexpectedMessage, CorruptMessage, etc. → SSLEOFError
                // Matches CPython's "EOF occurred in violation of protocol"
                SslError::Eof
            }
            rustls::Error::PeerIncompatible(peer_err) => {
                // Check for specific incompatibility types
                use rustls::PeerIncompatible;
                match peer_err {
                    PeerIncompatible::NoCipherSuitesInCommon => {
                        // Maps to OpenSSL SSL_R_NO_SHARED_CIPHER (lib=20, reason=193)
                        SslError::NoCipherSuites
                    }
                    _ => {
                        // Other protocol incompatibilities → SSLEOFError
                        SslError::Eof
                    }
                }
            }
            _ => SslError::Ssl(format!("{err}")),
        }
    }

    /// Create SSLError with library and reason from string values
    ///
    /// This is the base helper for creating SSLError with _library and _reason
    /// attributes when you already have the string values.
    ///
    /// # Arguments
    /// * `vm` - Virtual machine reference
    /// * `library` - Library name (e.g., "PEM", "SSL")
    /// * `reason` - Error reason (e.g., "PEM lib", "NO_SHARED_CIPHER")
    /// * `message` - Main error message
    ///
    /// # Returns
    /// PyBaseExceptionRef with _library and _reason attributes set
    ///
    /// # Note
    /// If attribute setting fails (extremely rare), returns the exception without attributes
    pub(super) fn create_ssl_error_with_reason(
        vm: &VirtualMachine,
        library: Option<&str>,
        reason: &str,
        message: impl Into<String>,
    ) -> PyBaseExceptionRef {
        let msg = message.into();
        // SSLError args should be (errno, message) format
        // FIXME: Use 1 as generic SSL error code
        let exc = vm.new_os_subtype_error(PySSLError::class(&vm.ctx).to_owned(), Some(1), msg);

        // Set library and reason attributes
        // Ignore errors as they're extremely rare (e.g., out of memory)
        let library_obj = match library {
            Some(lib) => vm.ctx.new_str(lib).as_object().to_owned(),
            None => vm.ctx.none(),
        };
        let _ = exc.as_object().set_attr("library", library_obj, vm);
        let _ =
            exc.as_object()
                .set_attr("reason", vm.ctx.new_str(reason).as_object().to_owned(), vm);

        exc.upcast()
    }

    /// Create SSLError with library and reason from ssl_data codes
    ///
    /// This helper converts OpenSSL numeric error codes to Python SSLError exceptions
    /// with proper _library and _reason attributes by looking up the error strings
    /// in ssl_data tables, then delegates to create_ssl_error_with_reason.
    ///
    /// # Arguments
    /// * `vm` - Virtual machine reference
    /// * `lib` - OpenSSL library code (e.g., ERR_LIB_SSL = 20)
    /// * `reason` - OpenSSL reason code (e.g., SSL_R_NO_SHARED_CIPHER = 193)
    ///
    /// # Returns
    /// PyBaseExceptionRef with _library and _reason attributes set
    fn create_ssl_error_from_codes(
        vm: &VirtualMachine,
        lib: i32,
        reason: i32,
    ) -> PyBaseExceptionRef {
        // Look up error strings from ssl_data tables
        let key = ssl_data::encode_error_key(lib, reason);
        let reason_str = ssl_data::ERROR_CODES
            .get(&key)
            .copied()
            .unwrap_or("unknown error");

        let lib_str = ssl_data::LIBRARY_CODES
            .get(&(lib as u32))
            .copied()
            .unwrap_or("UNKNOWN");

        // Delegate to create_ssl_error_with_reason for actual exception creation
        Self::create_ssl_error_with_reason(
            vm,
            Some(lib_str),
            reason_str,
            format!("[SSL] {reason_str}"),
        )
    }

    /// Convert to Python exception
    pub fn into_py_err(self, vm: &VirtualMachine) -> PyBaseExceptionRef {
        match self {
            SslError::WantRead => create_ssl_want_read_error(vm).upcast(),
            SslError::WantWrite => create_ssl_want_write_error(vm).upcast(),
            SslError::Timeout(msg) => timeout_error_msg(vm, msg).upcast(),
            SslError::Syscall(msg) => {
                // Create SSLError with library=None for syscall errors during SSL operations
                Self::create_ssl_error_with_reason(vm, None, &msg, msg.clone())
            }
            SslError::Ssl(msg) => vm
                .new_os_subtype_error(
                    PySSLError::class(&vm.ctx).to_owned(),
                    None,
                    format!("SSL error: {msg}"),
                )
                .upcast(),
            SslError::ZeroReturn => create_ssl_zero_return_error(vm).upcast(),
            SslError::Eof => create_ssl_eof_error(vm).upcast(),
            SslError::CertVerification(cert_err) => {
                // Use the proper cert verification error creator
                create_ssl_cert_verification_error(vm, &cert_err).expect("unlikely to happen")
            }
            SslError::Io(err) => err.into_pyexception(vm),
            SslError::SniCallbackRestart => {
                // This should be handled at PySSLSocket level
                unreachable!("SniCallbackRestart should not reach Python layer")
            }
            SslError::Py(exc) => exc,
            SslError::AlertReceived { lib, reason } => {
                Self::create_ssl_error_from_codes(vm, lib, reason)
            }
            SslError::NoCipherSuites => {
                // OpenSSL error: lib=20 (ERR_LIB_SSL), reason=193 (SSL_R_NO_SHARED_CIPHER)
                Self::create_ssl_error_from_codes(vm, ERR_LIB_SSL, SSL_R_NO_SHARED_CIPHER)
            }
        }
    }
}

pub type SslResult<T> = Result<T, SslError>;
/// Common protocol settings shared between client and server connections
#[derive(Debug)]
pub struct ProtocolSettings {
    pub versions: &'static [&'static rustls::SupportedProtocolVersion],
    pub kx_groups: Option<Vec<&'static dyn rustls::crypto::SupportedKxGroup>>,
    pub cipher_suites: Option<Vec<rustls::SupportedCipherSuite>>,
    pub alpn_protocols: Vec<Vec<u8>>,
}

/// Options for creating a server TLS configuration
#[derive(Debug)]
pub struct ServerConfigOptions {
    /// Common protocol settings (versions, ALPN, KX groups, cipher suites)
    pub protocol_settings: ProtocolSettings,
    /// Server certificate chain
    pub cert_chain: Vec<CertificateDer<'static>>,
    /// Server private key
    pub private_key: PrivateKeyDer<'static>,
    /// Root certificates for client verification (if required)
    pub root_store: Option<RootCertStore>,
    /// Whether to request client certificate
    pub request_client_cert: bool,
    /// Whether to use deferred client certificate validation (TLS 1.3)
    pub use_deferred_validation: bool,
    /// Custom certificate resolver (for SNI support)
    pub cert_resolver: Option<Arc<dyn ResolvesServerCert>>,
    /// Deferred certificate error storage (for TLS 1.3)
    pub deferred_cert_error: Option<Arc<ParkingRwLock<Option<String>>>>,
    /// Session storage for server-side session resumption
    pub session_storage: Option<Arc<rustls::server::ServerSessionMemoryCache>>,
    /// Shared ticketer for TLS 1.2 session tickets (stateless resumption)
    pub ticketer: Option<Arc<dyn rustls::server::ProducesTickets>>,
}

/// Options for creating a client TLS configuration
#[derive(Debug)]
pub struct ClientConfigOptions {
    /// Common protocol settings (versions, ALPN, KX groups, cipher suites)
    pub protocol_settings: ProtocolSettings,
    /// Root certificates for server verification
    pub root_store: Option<RootCertStore>,
    /// DER-encoded CA certificates (for partial chain verification)
    pub ca_certs_der: Vec<Vec<u8>>,
    /// Client certificate chain (for mTLS)
    pub cert_chain: Option<Vec<CertificateDer<'static>>>,
    /// Client private key (for mTLS)
    pub private_key: Option<PrivateKeyDer<'static>>,
    /// Whether to verify server certificates (CERT_NONE disables verification)
    pub verify_server_cert: bool,
    /// Whether to check hostname against certificate (check_hostname)
    pub check_hostname: bool,
    /// SSL verification flags (e.g., VERIFY_X509_STRICT)
    pub verify_flags: i32,
    /// Session store for client-side session resumption
    pub session_store: Option<Arc<dyn rustls::client::ClientSessionStore>>,
    /// Certificate Revocation Lists for CRL checking
    pub crls: Vec<CertificateRevocationListDer<'static>>,
}

/// Create custom CryptoProvider with specified cipher suites and key exchange groups
///
/// This helper function consolidates the duplicated CryptoProvider creation logic
/// for both server and client configurations.
fn create_custom_crypto_provider(
    cipher_suites: Option<Vec<rustls::SupportedCipherSuite>>,
    kx_groups: Option<Vec<&'static dyn rustls::crypto::SupportedKxGroup>>,
) -> Arc<rustls::crypto::CryptoProvider> {
    use rustls::crypto::aws_lc_rs::{ALL_CIPHER_SUITES, ALL_KX_GROUPS};
    let default_provider = rustls::crypto::aws_lc_rs::default_provider();

    Arc::new(rustls::crypto::CryptoProvider {
        cipher_suites: cipher_suites.unwrap_or_else(|| ALL_CIPHER_SUITES.to_vec()),
        kx_groups: kx_groups.unwrap_or_else(|| ALL_KX_GROUPS.to_vec()),
        signature_verification_algorithms: default_provider.signature_verification_algorithms,
        secure_random: default_provider.secure_random,
        key_provider: default_provider.key_provider,
    })
}

/// Create a server TLS configuration
///
/// This abstracts the complex rustls ServerConfig building logic,
/// matching SSL_CTX initialization for server sockets.
pub(super) fn create_server_config(options: ServerConfigOptions) -> Result<ServerConfig, String> {
    use rustls::server::WebPkiClientVerifier;

    // Ensure default CryptoProvider is installed
    ensure_default_provider();

    // Create custom crypto provider using helper function
    let custom_provider = create_custom_crypto_provider(
        options.protocol_settings.cipher_suites.clone(),
        options.protocol_settings.kx_groups.clone(),
    );

    // Step 1: Build the appropriate client cert verifier based on settings
    let client_cert_verifier: Option<Arc<dyn rustls::server::danger::ClientCertVerifier>> =
        if let Some(root_store) = options.root_store {
            if options.request_client_cert {
                // Client certificate verification required
                let base_verifier = WebPkiClientVerifier::builder(Arc::new(root_store))
                    .build()
                    .map_err(|e| format!("Failed to create client verifier: {e}"))?;

                if options.use_deferred_validation {
                    // TLS 1.3: Use deferred validation
                    if let Some(deferred_error) = options.deferred_cert_error {
                        use crate::ssl::cert::DeferredClientCertVerifier;
                        let deferred_verifier =
                            DeferredClientCertVerifier::new(base_verifier, deferred_error);
                        Some(Arc::new(deferred_verifier))
                    } else {
                        // No deferred error storage provided, use immediate validation
                        Some(base_verifier)
                    }
                } else {
                    // TLS 1.2 or non-deferred: Use immediate validation
                    Some(base_verifier)
                }
            } else {
                // No client authentication
                None
            }
        } else {
            // No root store - no client authentication
            None
        };

    // Step 2: Create ServerConfig builder once with the selected verifier
    let builder = ServerConfig::builder_with_provider(custom_provider.clone())
        .with_protocol_versions(options.protocol_settings.versions)
        .map_err(|e| format!("Failed to create server config builder: {e}"))?;

    let builder = if let Some(verifier) = client_cert_verifier {
        builder.with_client_cert_verifier(verifier)
    } else {
        builder.with_no_client_auth()
    };

    // Add certificate
    let mut config = if let Some(resolver) = options.cert_resolver {
        // Use custom cert resolver (e.g., for SNI)
        builder.with_cert_resolver(resolver)
    } else {
        // Use single certificate
        builder
            .with_single_cert(options.cert_chain, options.private_key)
            .map_err(|e| format!("Failed to set server certificate: {e}"))?
    };

    // Set ALPN protocols with fallback
    apply_alpn_with_fallback(
        &mut config.alpn_protocols,
        &options.protocol_settings.alpn_protocols,
    );

    // Set session storage for server-side session resumption (TLS 1.3)
    if let Some(session_storage) = options.session_storage {
        config.session_storage = session_storage;
    }

    // Set ticketer for TLS 1.2 session tickets (stateless resumption)
    if let Some(ticketer) = options.ticketer {
        config.ticketer = ticketer.clone();
    }

    Ok(config)
}

/// Build WebPki verifier with CRL support
///
/// This helper function consolidates the duplicated CRL setup logic for both
/// check_hostname=True and check_hostname=False cases.
fn build_webpki_verifier_with_crls(
    root_store: Arc<RootCertStore>,
    crls: Vec<CertificateRevocationListDer<'static>>,
    verify_flags: i32,
) -> Result<Arc<dyn rustls::client::danger::ServerCertVerifier>, String> {
    use rustls::client::WebPkiServerVerifier;

    let mut verifier_builder = WebPkiServerVerifier::builder(root_store);

    // Check if CRL verification is requested
    let crl_check_requested = verify_flags & X509_V_FLAG_CRL_CHECK != 0;
    let has_crls = !crls.is_empty();

    // Add CRLs if provided OR if CRL checking is explicitly requested
    // (even with empty CRLs, rustls will fail verification if CRL checking is enabled)
    if has_crls || crl_check_requested {
        verifier_builder = verifier_builder.with_crls(crls);

        // Check if we should only verify end-entity (leaf) certificates
        if verify_flags & X509_V_FLAG_CRL_CHECK != 0 {
            verifier_builder = verifier_builder.only_check_end_entity_revocation();
        }
    }

    let webpki_verifier = verifier_builder
        .build()
        .map_err(|e| format!("Failed to build WebPkiServerVerifier: {e}"))?;

    Ok(webpki_verifier as Arc<dyn rustls::client::danger::ServerCertVerifier>)
}

/// Apply verifier wrappers (CRLCheckVerifier and StrictCertVerifier)
///
/// This helper function consolidates the duplicated verifier wrapping logic.
fn apply_verifier_wrappers(
    verifier: Arc<dyn rustls::client::danger::ServerCertVerifier>,
    verify_flags: i32,
    has_crls: bool,
    ca_certs_der: Vec<Vec<u8>>,
) -> Arc<dyn rustls::client::danger::ServerCertVerifier> {
    let crl_check_requested = verify_flags & X509_V_FLAG_CRL_CHECK != 0;

    // Wrap with CRLCheckVerifier to enforce CRL checking when flags are set
    let verifier = if crl_check_requested {
        use crate::ssl::cert::CRLCheckVerifier;
        Arc::new(CRLCheckVerifier::new(
            verifier,
            has_crls,
            crl_check_requested,
        ))
    } else {
        verifier
    };

    // Always use PartialChainVerifier when trust store is not empty
    // This allows self-signed certificates in trust store to be trusted
    // (OpenSSL behavior: self-signed certs are always trusted, non-self-signed require flag)
    let verifier = if !ca_certs_der.is_empty() {
        use crate::ssl::cert::PartialChainVerifier;
        Arc::new(PartialChainVerifier::new(
            verifier,
            ca_certs_der,
            verify_flags,
        ))
    } else {
        verifier
    };

    // Wrap with StrictCertVerifier if VERIFY_X509_STRICT flag is set
    if verify_flags & VERIFY_X509_STRICT != 0 {
        Arc::new(super::cert::StrictCertVerifier::new(verifier, verify_flags))
    } else {
        verifier
    }
}

/// Apply ALPN protocols
///
/// OpenSSL 1.1.0f+ allows ALPN negotiation to fail without aborting handshake.
/// rustls follows RFC 7301 strictly and rejects connections with no matching protocol.
/// To emulate OpenSSL behavior, we add a special fallback protocol (null byte).
fn apply_alpn_with_fallback(config_alpn: &mut Vec<Vec<u8>>, alpn_protocols: &[Vec<u8>]) {
    if !alpn_protocols.is_empty() {
        *config_alpn = alpn_protocols.to_vec();
        config_alpn.push(vec![0u8]); // Add null byte as fallback marker
    }
}

/// Create a client TLS configuration
///
/// This abstracts the complex rustls ClientConfig building logic,
/// matching SSL_CTX initialization for client sockets.
pub(super) fn create_client_config(options: ClientConfigOptions) -> Result<ClientConfig, String> {
    // Ensure default CryptoProvider is installed
    ensure_default_provider();

    // Create custom crypto provider using helper function
    let custom_provider = create_custom_crypto_provider(
        options.protocol_settings.cipher_suites.clone(),
        options.protocol_settings.kx_groups.clone(),
    );

    // Step 1: Build the appropriate verifier based on verification settings
    let verifier: Arc<dyn rustls::client::danger::ServerCertVerifier> = if options
        .verify_server_cert
    {
        // Verify server certificates
        let root_store = options
            .root_store
            .ok_or("Root store required for server verification")?;

        let root_store_arc = Arc::new(root_store);

        // Check if root_store is empty (no CA certs loaded)
        // CPython allows this and fails during handshake with SSLCertVerificationError
        if root_store_arc.is_empty() {
            // Use EmptyRootStoreVerifier - always fails with UnknownIssuer during handshake
            use crate::ssl::cert::EmptyRootStoreVerifier;
            Arc::new(EmptyRootStoreVerifier)
        } else {
            // Calculate has_crls once for both hostname verification paths
            let has_crls = !options.crls.is_empty();

            if options.check_hostname {
                // Default behavior: verify both certificate chain and hostname
                let base_verifier = build_webpki_verifier_with_crls(
                    root_store_arc.clone(),
                    options.crls,
                    options.verify_flags,
                )?;

                // Apply CRL and Strict verifier wrappers using helper function
                apply_verifier_wrappers(
                    base_verifier,
                    options.verify_flags,
                    has_crls,
                    options.ca_certs_der.clone(),
                )
            } else {
                // check_hostname=False: verify certificate chain but ignore hostname
                use crate::ssl::cert::HostnameIgnoringVerifier;

                // Build verifier with CRL support using helper function
                let webpki_verifier = build_webpki_verifier_with_crls(
                    root_store_arc.clone(),
                    options.crls,
                    options.verify_flags,
                )?;

                // Apply CRL verifier wrapper if needed (without Strict wrapper yet)
                let crl_check_requested = options.verify_flags & X509_V_FLAG_CRL_CHECK != 0;
                let verifier = if crl_check_requested {
                    use crate::ssl::cert::CRLCheckVerifier;
                    Arc::new(CRLCheckVerifier::new(
                        webpki_verifier,
                        has_crls,
                        crl_check_requested,
                    )) as Arc<dyn rustls::client::danger::ServerCertVerifier>
                } else {
                    webpki_verifier
                };

                // Wrap with PartialChainVerifier if VERIFY_X509_PARTIAL_CHAIN is set
                const VERIFY_X509_PARTIAL_CHAIN: i32 = 0x80000;
                let verifier = if options.verify_flags & VERIFY_X509_PARTIAL_CHAIN != 0 {
                    use crate::ssl::cert::PartialChainVerifier;
                    Arc::new(PartialChainVerifier::new(
                        verifier,
                        options.ca_certs_der.clone(),
                        options.verify_flags,
                    )) as Arc<dyn rustls::client::danger::ServerCertVerifier>
                } else {
                    verifier
                };

                // Wrap with HostnameIgnoringVerifier to bypass hostname checking
                let hostname_ignoring_verifier: Arc<
                    dyn rustls::client::danger::ServerCertVerifier,
                > = Arc::new(HostnameIgnoringVerifier::new_with_verifier(verifier));

                // Apply Strict verifier wrapper once at the end if needed
                if options.verify_flags & VERIFY_X509_STRICT != 0 {
                    Arc::new(crate::ssl::cert::StrictCertVerifier::new(
                        hostname_ignoring_verifier,
                        options.verify_flags,
                    ))
                } else {
                    hostname_ignoring_verifier
                }
            }
        }
    } else {
        // CERT_NONE: disable all verification
        use crate::ssl::cert::NoVerifier;
        Arc::new(NoVerifier)
    };

    // Step 2: Create ClientConfig builder once with the selected verifier
    let builder = ClientConfig::builder_with_provider(custom_provider.clone())
        .with_protocol_versions(options.protocol_settings.versions)
        .map_err(|e| format!("Failed to create client config builder: {e}"))?
        .dangerous()
        .with_custom_certificate_verifier(verifier);

    // Add client certificate if provided (mTLS)
    let mut config =
        if let (Some(cert_chain), Some(private_key)) = (options.cert_chain, options.private_key) {
            builder
                .with_client_auth_cert(cert_chain, private_key)
                .map_err(|e| format!("Failed to set client certificate: {e}"))?
        } else {
            builder.with_no_client_auth()
        };

    // Set ALPN protocols
    apply_alpn_with_fallback(
        &mut config.alpn_protocols,
        &options.protocol_settings.alpn_protocols,
    );

    // Set session resumption
    if let Some(session_store) = options.session_store {
        use rustls::client::Resumption;
        config.resumption = Resumption::store(session_store);
    }

    Ok(config)
}

/// Helper function - check if error is BlockingIOError
pub(super) fn is_blocking_io_error(err: &Py<PyBaseException>, vm: &VirtualMachine) -> bool {
    err.fast_isinstance(vm.ctx.exceptions.blocking_io_error)
}

// Handshake Helper Functions

/// Write TLS handshake data to socket/BIO
///
/// Drains all pending TLS data from rustls and sends it to the peer.
/// Returns whether any progress was made.
fn handshake_write_loop(
    conn: &mut TlsConnection,
    socket: &PySSLSocket,
    force_initial_write: bool,
    vm: &VirtualMachine,
) -> SslResult<bool> {
    let mut made_progress = false;

    while conn.wants_write() || force_initial_write {
        if force_initial_write && !conn.wants_write() {
            // No data to write on first iteration - break to avoid infinite loop
            break;
        }

        let mut buf = Vec::new();
        let written = conn
            .write_tls(&mut buf as &mut dyn std::io::Write)
            .map_err(SslError::Io)?;

        if written > 0 && !buf.is_empty() {
            // Send directly without select - blocking sockets will wait automatically
            // Handle BlockingIOError from non-blocking sockets
            match socket.sock_send(buf, vm) {
                Ok(_) => {
                    made_progress = true;
                }
                Err(e) => {
                    if is_blocking_io_error(&e, vm) {
                        // Non-blocking socket would block - return SSLWantWriteError
                        return Err(SslError::WantWrite);
                    }
                    return Err(SslError::Py(e));
                }
            }
        } else if written == 0 {
            // No data written but wants_write is true - should not happen normally
            // Break to avoid infinite loop
            break;
        }

        // Check if there's more to write
        if !conn.wants_write() {
            break;
        }
    }

    Ok(made_progress)
}

/// Read TLS handshake data from socket/BIO
///
/// Waits for and reads TLS records from the peer, handling SNI callback setup.
/// Returns (made_progress, is_first_sni_read).
fn handshake_read_data(
    conn: &mut TlsConnection,
    socket: &PySSLSocket,
    is_bio: bool,
    is_server: bool,
    vm: &VirtualMachine,
) -> SslResult<(bool, bool)> {
    if !conn.wants_read() {
        return Ok((false, false));
    }

    // SERVER-SPECIFIC: Check if this is the first read (for SNI callback)
    // Must check BEFORE reading data, so we can detect first time
    let is_first_sni_read = is_server && socket.is_first_sni_read();

    // Wait for data in socket mode
    if !is_bio {
        let timed_out = socket
            .sock_wait_for_io_impl(SelectKind::Read, vm)
            .map_err(SslError::Py)?;

        if timed_out {
            // This should rarely happen now - only if socket itself has a timeout
            // and we're waiting for required handshake data
            return Err(SslError::Timeout("timed out".to_string()));
        }
    }

    let data_obj = match socket.sock_recv(SSL3_RT_MAX_PLAIN_LENGTH, vm) {
        Ok(d) => d,
        Err(e) => {
            if is_blocking_io_error(&e, vm) {
                return Err(SslError::WantRead);
            }
            // In socket mode, if recv times out and we're only waiting for read
            // (no wants_write), we might be waiting for optional NewSessionTicket in TLS 1.3
            // Consider the handshake complete
            if !is_bio && !conn.wants_write() {
                // Check if it's a timeout exception
                if e.fast_isinstance(vm.ctx.exceptions.timeout_error) {
                    // Timeout waiting for optional data - handshake is complete
                    return Ok((false, false));
                }
            }
            return Err(SslError::Py(e));
        }
    };

    // SERVER-SPECIFIC: Save ClientHello on first read for potential connection recreation
    if is_first_sni_read {
        // Extract bytes from PyObjectRef
        use rustpython_vm::builtins::PyBytes;
        if let Some(bytes_obj) = data_obj.downcast_ref::<PyBytes>() {
            socket.save_client_hello_from_bytes(bytes_obj.as_bytes());
        }
    }

    // Feed data to rustls
    ssl_read_tls_records(conn, data_obj, is_bio, vm)?;

    Ok((true, is_first_sni_read))
}

/// Handle handshake completion for server-side TLS 1.3
///
/// Tries to send NewSessionTicket in non-blocking mode to avoid deadlocks.
/// Returns true if handshake is complete and we should exit.
fn handle_handshake_complete(
    conn: &mut TlsConnection,
    socket: &PySSLSocket,
    _is_server: bool,
    vm: &VirtualMachine,
) -> SslResult<bool> {
    if conn.is_handshaking() {
        return Ok(false); // Not complete yet
    }

    // Handshake is complete!
    //
    // Different behavior for BIO mode vs socket mode:
    //
    // BIO mode (CPython-compatible):
    // - Python code calls outgoing.read() to get pending data
    // - We just return here and let Python handle the data
    //
    // Socket mode (rustls-specific):
    // - OpenSSL automatically writes to socket in SSL_do_handshake()
    // - We must explicitly call write_tls() to send pending data
    // - Without this, client hangs waiting for server's NewSessionTicket

    if socket.is_bio_mode() {
        // BIO mode: Write pending data to outgoing BIO (one-time drain)
        // Python's ssl_io_loop will read from outgoing BIO
        if conn.wants_write() {
            // Call write_tls ONCE to drain pending data
            // Do NOT loop on wants_write() - avoid infinite loop/deadlock
            let tls_data = ssl_write_tls_records(conn)?;
            if !tls_data.is_empty() {
                socket.sock_send(tls_data, vm).map_err(SslError::Py)?;
            }

            // IMPORTANT: Don't check wants_write() again!
            // Python's ssl_io_loop will call do_handshake() again if needed
        }
    } else if conn.wants_write() {
        // Send all pending data (e.g., TLS 1.3 NewSessionTicket) to socket
        while conn.wants_write() {
            let tls_data = ssl_write_tls_records(conn)?;
            if tls_data.is_empty() {
                break;
            }
            socket.sock_send(tls_data, vm).map_err(SslError::Py)?;
        }
    }

    Ok(true)
}

/// Try to read plaintext data from TLS connection buffer
///
/// Returns Ok(Some(n)) if n bytes were read, Ok(None) if would block,
/// or Err on real errors.
fn try_read_plaintext(conn: &mut TlsConnection, buf: &mut [u8]) -> SslResult<Option<usize>> {
    let mut reader = conn.reader();
    match reader.read(buf) {
        Ok(0) => {
            // EOF from TLS connection
            Ok(Some(0))
        }
        Ok(n) => {
            // Successfully read n bytes
            Ok(Some(n))
        }
        Err(e) if e.kind() != std::io::ErrorKind::WouldBlock => {
            // Real error
            Err(SslError::Io(e))
        }
        Err(_) => {
            // WouldBlock - no plaintext available
            Ok(None)
        }
    }
}

/// Equivalent to OpenSSL's SSL_do_handshake()
///
/// Performs TLS handshake by exchanging data with the peer until completion.
/// This abstracts away the low-level rustls read_tls/write_tls loop.
///
/// = SSL_do_handshake()
pub(super) fn ssl_do_handshake(
    conn: &mut TlsConnection,
    socket: &PySSLSocket,
    vm: &VirtualMachine,
) -> SslResult<()> {
    // Check if handshake is already done
    if !conn.is_handshaking() {
        return Ok(());
    }

    let is_bio = socket.is_bio_mode();
    let is_server = matches!(conn, TlsConnection::Server(_));
    let mut first_iteration = true; // Track if this is the first loop iteration
    let mut iteration_count = 0;

    loop {
        iteration_count += 1;
        let mut made_progress = false;

        // IMPORTANT: In BIO mode, force initial write even if wants_write() is false
        // rustls requires write_tls() to be called to generate ClientHello/ServerHello
        let force_initial_write = is_bio && first_iteration;

        // Write TLS handshake data to socket/BIO
        let write_progress = handshake_write_loop(conn, socket, force_initial_write, vm)?;
        made_progress |= write_progress;

        // Read TLS handshake data from socket/BIO
        let (read_progress, is_first_sni_read) =
            handshake_read_data(conn, socket, is_bio, is_server, vm)?;
        made_progress |= read_progress;

        // Process TLS packets (state machine)
        if let Err(e) = conn.process_new_packets() {
            // Send close_notify on error
            if !is_bio {
                conn.send_close_notify();
                // Actually send the close_notify alert
                if let Ok(alert_data) = ssl_write_tls_records(conn)
                    && !alert_data.is_empty()
                {
                    let _ = socket.sock_send(alert_data, vm);
                }
            }

            // Certificate verification errors are already handled by from_rustls

            return Err(SslError::from_rustls(e));
        }

        // SERVER-SPECIFIC: Check SNI callback after processing packets
        // SNI name is extracted during process_new_packets()
        // Invoke callback on FIRST read if callback is configured, regardless of SNI presence
        if is_server && is_first_sni_read && socket.has_sni_callback() {
            // IMPORTANT: Do NOT call the callback here!
            // The connection lock is still held, which would cause deadlock.
            // Return SniCallbackRestart to signal do_handshake to:
            // 1. Drop conn_guard
            // 2. Call the callback (with Some(name) or None)
            // 3. Restart handshake
            return Err(SslError::SniCallbackRestart);
        }

        // Check if handshake is complete and handle post-handshake processing
        // CRITICAL: We do NOT check wants_read() - this matches CPython/OpenSSL behavior!
        if handle_handshake_complete(conn, socket, is_server, vm)? {
            return Ok(());
        }

        // In BIO mode, stop after one iteration
        if is_bio {
            // Before returning WANT error, write any pending TLS data to BIO
            // This is critical: if wants_write is true after process_new_packets,
            // we need to write that data to the outgoing BIO before returning
            if conn.wants_write() {
                // Write all pending TLS data to outgoing BIO
                loop {
                    let mut buf = vec![0u8; SSL3_RT_MAX_PLAIN_LENGTH];
                    let n = match conn.write_tls(&mut buf.as_mut_slice()) {
                        Ok(n) => n,
                        Err(_) => break,
                    };
                    if n == 0 {
                        break;
                    }
                    // Send to outgoing BIO
                    socket
                        .sock_send(buf[..n].to_vec(), vm)
                        .map_err(SslError::Py)?;
                    // Check if there's more to write
                    if !conn.wants_write() {
                        break;
                    }
                }
                // After writing, check if we still want more
                // If all data was written, wants_write may now be false
                if conn.wants_write() {
                    // Still need more - return WANT_WRITE
                    return Err(SslError::WantWrite);
                }
                // Otherwise fall through to check wants_read
            }

            // Check if we need to read
            if conn.wants_read() {
                return Err(SslError::WantRead);
            }
            break;
        }

        // Mark that we've completed the first iteration
        first_iteration = false;

        // Improved loop termination logic:
        // Continue looping if:
        // 1. Rustls wants more I/O (wants_read or wants_write), OR
        // 2. We made progress in this iteration
        //
        // This is more robust than just checking made_progress, because:
        // - Rustls may need multiple iterations to process TLS state machine
        // - Network delays may cause temporary "no progress" situations
        // - wants_read/wants_write accurately reflect Rustls internal state
        let should_continue = conn.wants_read() || conn.wants_write() || made_progress;

        if !should_continue {
            break;
        }

        // Safety check: prevent truly infinite loops (should never happen)
        if iteration_count > 1000 {
            break;
        }
    }

    // If we exit the loop without completing handshake, return error
    // Check rustls state to provide better error message
    if conn.is_handshaking() {
        Err(SslError::Syscall(format!(
            "SSL handshake failed: incomplete after {iteration_count} iterations",
        )))
    } else {
        // Handshake completed successfully (shouldn't reach here normally)
        Ok(())
    }
}

/// Equivalent to OpenSSL's SSL_read()
///
/// Reads application data from TLS connection.
/// Automatically handles TLS record I/O as needed.
///
/// = SSL_read_ex()
pub(super) fn ssl_read(
    conn: &mut TlsConnection,
    buf: &mut [u8],
    socket: &PySSLSocket,
    vm: &VirtualMachine,
) -> SslResult<usize> {
    let is_bio = socket.is_bio_mode();

    // Get socket timeout and calculate deadline (= _PyDeadline_Init)
    let deadline = if !is_bio {
        match socket.get_socket_timeout(vm).map_err(SslError::Py)? {
            Some(timeout) if !timeout.is_zero() => Some(std::time::Instant::now() + timeout),
            _ => None, // None = blocking (no deadline), Some(0) = non-blocking (handled below)
        }
    } else {
        None // BIO mode has no deadline
    };

    // Loop to handle TLS records and post-handshake messages
    // Matches SSL_read behavior which loops until data is available
    //   - CPython uses OpenSSL's SSL_read which loops on SSL_ERROR_WANT_READ/WANT_WRITE
    //   - We use rustls which requires manual read_tls/process_new_packets loop
    //   - No iteration limit: relies on deadline and blocking I/O
    //   - Blocking sockets: sock_select() and recv() wait at kernel level (no CPU busy-wait)
    //   - Non-blocking sockets: immediate return on first WantRead
    //   - Deadline prevents timeout issues
    loop {
        // Check deadline
        if let Some(deadline) = deadline
            && std::time::Instant::now() >= deadline
        {
            // Timeout expired
            return Err(SslError::Timeout(
                "The read operation timed out".to_string(),
            ));
        }
        // Check if we need to read more TLS records BEFORE trying plaintext read
        // This ensures we don't miss data that's already been processed
        let needs_more_tls = conn.wants_read();

        // Try to read plaintext from rustls buffer
        if let Some(n) = try_read_plaintext(conn, buf)? {
            return Ok(n);
        }

        // No plaintext available and rustls doesn't want to read more TLS records
        if !needs_more_tls {
            // Check if connection needs to write data first (e.g., TLS key update, renegotiation)
            // This mirrors the handshake logic which checks both wants_read() and wants_write()
            if conn.wants_write() && !is_bio {
                // Flush pending TLS data before continuing
                let tls_data = ssl_write_tls_records(conn)?;
                if !tls_data.is_empty() {
                    socket.sock_send(tls_data, vm).map_err(SslError::Py)?;
                }
                // After flushing, rustls may want to read again - continue loop
                continue;
            }

            // BIO mode: check for EOF
            if is_bio && let Some(bio_obj) = socket.incoming_bio() {
                let is_eof = bio_obj
                    .get_attr("eof", vm)
                    .and_then(|v| v.try_into_value::<bool>(vm))
                    .unwrap_or(false);
                if is_eof {
                    return Err(SslError::Eof);
                }
            }
            return Err(SslError::WantRead);
        }

        // Read and process TLS records
        // This will block for blocking sockets
        match ssl_ensure_data_available(conn, socket, vm) {
            Ok(_bytes_read) => {
                // Successfully read and processed TLS data
                // Continue loop to try reading plaintext
            }
            Err(SslError::Io(ref io_err)) if io_err.to_string().contains("message buffer full") => {
                // Buffer is full - we need to consume plaintext before reading more
                // Try to read plaintext now
                match try_read_plaintext(conn, buf)? {
                    Some(n) if n > 0 => {
                        // Have plaintext - return it
                        // Python will call read() again if it needs more data
                        return Ok(n);
                    }
                    _ => {
                        // No plaintext available yet - this is unusual
                        // Return WantRead to let Python retry
                        return Err(SslError::WantRead);
                    }
                }
            }
            Err(e) => {
                // Other errors - check for buffered plaintext before propagating
                match try_read_plaintext(conn, buf)? {
                    Some(n) if n > 0 => {
                        // Have buffered plaintext - return it successfully
                        return Ok(n);
                    }
                    _ => {
                        // No buffered data - propagate the error
                        return Err(e);
                    }
                }
            }
        }
    }
}

// Helper functions (private-ish, used by public SSL functions)

/// Write TLS records from rustls to socket
fn ssl_write_tls_records(conn: &mut TlsConnection) -> SslResult<Vec<u8>> {
    let mut buf = Vec::new();
    let n = conn
        .write_tls(&mut buf as &mut dyn std::io::Write)
        .map_err(SslError::Io)?;

    if n > 0 { Ok(buf) } else { Ok(Vec::new()) }
}

/// Read TLS records from socket to rustls
fn ssl_read_tls_records(
    conn: &mut TlsConnection,
    data: PyObjectRef,
    is_bio: bool,
    vm: &VirtualMachine,
) -> SslResult<()> {
    // Convert PyObject to bytes-like (supports bytes, bytearray, etc.)
    let bytes = ArgBytesLike::try_from_object(vm, data)
        .map_err(|_| SslError::Syscall("Expected bytes-like object".to_string()))?;

    let bytes_data = bytes.borrow_buf();

    if bytes_data.is_empty() {
        // different error for BIO vs socket mode
        if is_bio {
            // In BIO mode, no data means WANT_READ
            return Err(SslError::WantRead);
        } else {
            // In socket mode, empty recv() means TCP EOF (FIN received)
            // Need to distinguish:
            // 1. Clean shutdown: received TLS close_notify → return ZeroReturn (0 bytes)
            // 2. Unexpected EOF: no close_notify → return Eof (SSLEOFError)
            //
            // SSL_ERROR_ZERO_RETURN vs SSL_ERROR_SYSCALL(errno=0) logic
            // CPython checks SSL_get_shutdown() & SSL_RECEIVED_SHUTDOWN
            //
            // Process any buffered TLS records (may contain close_notify)
            let _ = conn.process_new_packets();

            // IMPORTANT: CPython's default behavior (suppress_ragged_eofs=True)
            // treats empty recv() as clean shutdown, returning 0 bytes instead of raising SSLEOFError.
            //
            // This is necessary for HTTP/1.0 servers that:
            // 1. Send response without Content-Length header
            // 2. Signal end-of-response by closing connection (TCP FIN)
            // 3. Don't send TLS close_notify before TCP close
            //
            // While this could theoretically allow truncation attacks,
            // it's the standard behavior for compatibility with real-world servers.
            // Python only raises SSLEOFError when suppress_ragged_eofs=False is explicitly set.
            //
            // TODO: Implement suppress_ragged_eofs parameter if needed for strict security mode.
            return Err(SslError::ZeroReturn);
        }
    }

    // Feed all received data to read_tls - loop to consume all data
    // read_tls may not consume all data in one call
    let mut offset = 0;
    while offset < bytes_data.len() {
        let remaining = &bytes_data[offset..];
        let mut cursor = std::io::Cursor::new(remaining);

        match conn.read_tls(&mut cursor) {
            Ok(read_bytes) => {
                if read_bytes == 0 {
                    // No more data can be consumed
                    break;
                }
                offset += read_bytes;
            }
            Err(e) => {
                // Real error - propagate it
                return Err(SslError::Io(e));
            }
        }
    }

    Ok(())
}

/// Check if an exception is a connection closed error
/// In SSL context, these errors indicate unexpected connection termination without proper TLS shutdown
fn is_connection_closed_error(exc: &Py<PyBaseException>, vm: &VirtualMachine) -> bool {
    use rustpython_vm::stdlib::errno::errors;

    // Check for ConnectionAbortedError, ConnectionResetError (Python exception types)
    if exc.fast_isinstance(vm.ctx.exceptions.connection_aborted_error)
        || exc.fast_isinstance(vm.ctx.exceptions.connection_reset_error)
    {
        return true;
    }

    // Also check OSError with specific errno values (ECONNABORTED, ECONNRESET)
    if exc.fast_isinstance(vm.ctx.exceptions.os_error)
        && let Ok(errno) = exc.as_object().get_attr("errno", vm)
        && let Ok(errno_int) = errno.try_int(vm)
        && let Ok(errno_val) = errno_int.try_to_primitive::<i32>(vm)
    {
        return errno_val == errors::ECONNABORTED || errno_val == errors::ECONNRESET;
    }
    false
}

/// Ensure TLS data is available for reading
/// Returns the number of bytes read from the socket
fn ssl_ensure_data_available(
    conn: &mut TlsConnection,
    socket: &PySSLSocket,
    vm: &VirtualMachine,
) -> SslResult<usize> {
    // Unlike OpenSSL's SSL_read, rustls requires explicit I/O
    if conn.wants_read() {
        let is_bio = socket.is_bio_mode();

        // For non-BIO mode (regular sockets), check if socket is ready first
        // PERFORMANCE OPTIMIZATION: Only use select for sockets with timeout
        // - Blocking sockets (timeout=None): Skip select, recv() will block naturally
        // - Timeout sockets: Use select to enforce timeout
        // - Non-blocking sockets: Skip select, recv() will return EAGAIN immediately
        if !is_bio {
            let timeout = socket.get_socket_timeout(vm).map_err(SslError::Py)?;

            // Only use select if socket has a positive timeout
            if let Some(t) = timeout
                && !t.is_zero()
            {
                // Socket has timeout - use select to enforce it
                let timed_out = socket
                    .sock_wait_for_io_impl(SelectKind::Read, vm)
                    .map_err(SslError::Py)?;
                if timed_out {
                    // Socket not ready within timeout
                    return Err(SslError::WantRead);
                }
            }
            // else: non-blocking socket (timeout=0) or blocking socket (timeout=None) - skip select
        }

        let data = match socket.sock_recv(2048, vm) {
            Ok(data) => data,
            Err(e) => {
                // Before returning socket error, check if rustls already has a queued TLS alert
                // This mirrors CPython/OpenSSL behavior: SSL errors take precedence over socket errors
                // On Windows, TCP RST may arrive before we read the alert, but rustls may have
                // already received and buffered the alert from a previous read
                if let Err(rustls_err) = conn.process_new_packets() {
                    return Err(SslError::from_rustls(rustls_err));
                }
                // In SSL context, connection closed errors (ECONNABORTED, ECONNRESET) indicate
                // unexpected connection termination - the peer closed without proper TLS shutdown.
                // This is semantically equivalent to "EOF occurred in violation of protocol"
                // because no close_notify alert was received.
                // On Windows, TCP RST can arrive before we read the TLS alert, causing these errors.
                if is_connection_closed_error(&e, vm) {
                    return Err(SslError::Eof);
                }
                return Err(SslError::Py(e));
            }
        };

        // Get the size of received data
        let bytes_read = data
            .clone()
            .try_into_value::<rustpython_vm::builtins::PyBytes>(vm)
            .map(|b| b.as_bytes().len())
            .unwrap_or(0);

        // Check if BIO has EOF set (incoming BIO closed)
        let is_eof = if is_bio {
            // Check incoming BIO's eof property
            if let Some(bio_obj) = socket.incoming_bio() {
                bio_obj
                    .get_attr("eof", vm)
                    .and_then(|v| v.try_into_value::<bool>(vm))
                    .unwrap_or(false)
            } else {
                false
            }
        } else {
            false
        };

        // If BIO EOF is set and no data available, treat as connection EOF
        if is_eof && bytes_read == 0 {
            return Err(SslError::Eof);
        }

        // Feed data to rustls and process packets
        ssl_read_tls_records(conn, data, is_bio, vm)?;

        // Process any packets we successfully read
        conn.process_new_packets().map_err(SslError::from_rustls)?;

        Ok(bytes_read)
    } else {
        // No data to read
        Ok(0)
    }
}

// Multi-Certificate Resolver for RSA/ECC Support

/// Multi-certificate resolver that selects appropriate certificate based on client capabilities
///
/// This resolver implements OpenSSL's behavior of supporting multiple certificate/key pairs
/// (e.g., one RSA and one ECC) and selecting the appropriate one based on the client's
/// supported signature algorithms during the TLS handshake.
///
/// OpenSSL's SSL_CTX_use_certificate_chain_file can be called multiple
/// times to add different certificate types, and OpenSSL automatically selects the best one.
#[derive(Debug)]
pub(super) struct MultiCertResolver {
    cert_keys: Vec<Arc<CertifiedKey>>,
}

impl MultiCertResolver {
    /// Create a new multi-certificate resolver
    pub fn new(cert_keys: Vec<Arc<CertifiedKey>>) -> Self {
        Self { cert_keys }
    }
}

impl ResolvesServerCert for MultiCertResolver {
    fn resolve(&self, client_hello: rustls::server::ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        // Get the signature schemes supported by the client
        let client_schemes = client_hello.signature_schemes();

        // Try to find a certificate that matches the client's signature schemes
        for cert_key in &self.cert_keys {
            // Check if this certificate's signing key is compatible with any of the
            // client's supported signature schemes
            if let Some(_scheme) = cert_key.key.choose_scheme(client_schemes) {
                return Some(cert_key.clone());
            }
        }

        // If no perfect match, return the first certificate as fallback
        // (This matches OpenSSL's behavior of using the first loaded cert if negotiation fails)
        self.cert_keys.first().cloned()
    }
}

// Helper Functions for OpenSSL Compatibility:

/// Normalize cipher suite name for OpenSSL compatibility
///
/// Converts rustls cipher names to OpenSSL format:
/// - TLS_AES_256_GCM_SHA384 → AES256-GCM-SHA384
/// - Replaces "AES-256" with "AES256" and "AES-128" with "AES128"
pub(super) fn normalize_cipher_name(rustls_name: &str) -> String {
    rustls_name
        .strip_prefix("TLS_")
        .unwrap_or(rustls_name)
        .replace("_WITH_", "_")
        .replace('_', "-")
        .replace("AES-256", "AES256")
        .replace("AES-128", "AES128")
}

/// Get cipher key size in bits from cipher suite name
///
/// Returns:
/// - 256 for AES-256 and ChaCha20
/// - 128 for AES-128
/// - 0 for unknown ciphers
pub(super) fn get_cipher_key_bits(cipher_name: &str) -> i32 {
    if cipher_name.contains("256") || cipher_name.contains("CHACHA20") {
        256
    } else if cipher_name.contains("128") {
        128
    } else {
        0
    }
}

/// Get encryption algorithm description from cipher name
///
/// Returns human-readable encryption description for OpenSSL compatibility
pub(super) fn get_cipher_encryption_desc(cipher_name: &str) -> &'static str {
    if cipher_name.contains("AES256") {
        "AESGCM(256)"
    } else if cipher_name.contains("AES128") {
        "AESGCM(128)"
    } else if cipher_name.contains("CHACHA20") {
        "CHACHA20-POLY1305(256)"
    } else {
        "Unknown"
    }
}

/// Normalize rustls cipher suite name to IANA standard format
///
/// Converts rustls Debug format names to IANA standard:
/// - "TLS13_AES_256_GCM_SHA384" -> "TLS_AES_256_GCM_SHA384"
/// - Other names remain unchanged
pub(super) fn normalize_rustls_cipher_name(rustls_name: &str) -> String {
    if rustls_name.starts_with("TLS13_") {
        rustls_name.replace("TLS13_", "TLS_")
    } else {
        rustls_name.to_string()
    }
}

/// Convert rustls protocol version to string representation
///
/// Returns the TLS version string
/// - TLSv1.2, TLSv1.3, or "Unknown"
pub(super) fn get_protocol_version_str(version: &rustls::SupportedProtocolVersion) -> &'static str {
    match version.version {
        rustls::ProtocolVersion::TLSv1_2 => "TLSv1.2",
        rustls::ProtocolVersion::TLSv1_3 => "TLSv1.3",
        _ => "Unknown",
    }
}

/// Cipher suite information
///
/// Contains all relevant cipher information extracted from a rustls CipherSuite
pub(super) struct CipherInfo {
    /// IANA standard cipher name (e.g., "TLS_AES_256_GCM_SHA384")
    pub name: String,
    /// TLS protocol version (e.g., "TLSv1.2", "TLSv1.3")
    pub protocol: &'static str,
    /// Key size in bits (e.g., 128, 256)
    pub bits: i32,
}

/// Extract cipher information from a rustls CipherSuite
///
/// This consolidates the common cipher extraction logic used across
/// get_ciphers(), cipher(), and shared_ciphers() methods.
pub(super) fn extract_cipher_info(suite: &rustls::SupportedCipherSuite) -> CipherInfo {
    let rustls_name = format!("{:?}", suite.suite());
    let name = normalize_rustls_cipher_name(&rustls_name);
    let protocol = get_protocol_version_str(suite.version());
    let bits = get_cipher_key_bits(&name);

    CipherInfo {
        name,
        protocol,
        bits,
    }
}

/// Convert curve name to rustls key exchange group
///
/// Maps OpenSSL curve names (e.g., "prime256v1", "secp384r1") to rustls KxGroups.
/// Returns an error if the curve is not supported by rustls.
pub(super) fn curve_name_to_kx_group(
    curve: &str,
) -> Result<Vec<&'static dyn SupportedKxGroup>, String> {
    // Get the default crypto provider's key exchange groups
    let provider = rustls::crypto::aws_lc_rs::default_provider();
    let all_groups = &provider.kx_groups;

    match curve {
        // P-256 (also known as secp256r1 or prime256v1)
        "prime256v1" | "secp256r1" => {
            // Find SECP256R1 in the provider's groups
            all_groups
                .iter()
                .find(|g| g.name() == rustls::NamedGroup::secp256r1)
                .map(|g| vec![*g])
                .ok_or_else(|| "secp256r1 not supported by crypto provider".to_owned())
        }
        // P-384 (also known as secp384r1 or prime384v1)
        "secp384r1" | "prime384v1" => all_groups
            .iter()
            .find(|g| g.name() == rustls::NamedGroup::secp384r1)
            .map(|g| vec![*g])
            .ok_or_else(|| "secp384r1 not supported by crypto provider".to_owned()),
        // X25519
        "X25519" | "x25519" => all_groups
            .iter()
            .find(|g| g.name() == rustls::NamedGroup::X25519)
            .map(|g| vec![*g])
            .ok_or_else(|| "X25519 not supported by crypto provider".to_owned()),
        // P-521 (also known as secp521r1 or prime521v1)
        // Now supported with aws-lc-rs crypto provider
        "prime521v1" | "secp521r1" => all_groups
            .iter()
            .find(|g| g.name() == rustls::NamedGroup::secp521r1)
            .map(|g| vec![*g])
            .ok_or_else(|| "secp521r1 not supported by crypto provider".to_owned()),
        // X448
        // Now supported with aws-lc-rs crypto provider
        "X448" | "x448" => all_groups
            .iter()
            .find(|g| g.name() == rustls::NamedGroup::X448)
            .map(|g| vec![*g])
            .ok_or_else(|| "X448 not supported by crypto provider".to_owned()),
        _ => Err(format!("unknown curve name '{curve}'")),
    }
}
