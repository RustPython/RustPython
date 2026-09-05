// spell-checker: ignore webpki ssleof sslerror akid certsign sslerr aesgcm

// OpenSSL compatibility layer for rustls
//
// This module provides OpenSSL-like abstractions over rustls APIs,
// making the code more readable and maintainable. Each function is named
// after its OpenSSL equivalent (e.g., ssl_do_handshake corresponds to SSL_do_handshake).

// SSL error code data tables (shared with OpenSSL backend for compatibility)
// These map OpenSSL error codes to human-readable strings
#[allow(
    clippy::duplicate_mod,
    reason = "This is duplicated only when running clippy. The two features are mutually exclusive"
)]
#[path = "../openssl/ssl_data_31.rs"]
mod ssl_data;

use crate::socket::{SockWaitKind, timeout_error_msg};
use crate::vm::VirtualMachine;
use alloc::sync::Arc;
use parking_lot::RwLock as ParkingRwLock;
use rustls::Connection;
use rustls::client::ClientConfig;
use rustls::crypto::{CryptoProvider, SupportedKxGroup};
use rustls::pki_types::{CertificateDer, CertificateRevocationListDer, PrivateKeyDer};
use rustls::server::{ProducesTickets, ResolvesServerCert, ServerConfig, WebPkiClientVerifier};
use rustls::sign::CertifiedKey;
use rustls::{RootCertStore, SupportedCipherSuite};
use rustpython_vm::builtins::{PyBaseException, PyBaseExceptionRef};
use rustpython_vm::convert::IntoPyException;
use rustpython_vm::function::ArgBytesLike;
use rustpython_vm::{AsObject, Py, PyObjectRef, PyPayload, PyResult, TryFromObject};
use std::io::Read;

use super::chain::{self, Purpose, VerifiedChainBuilder};
use super::providers::CryptoExt;

// Import PySSLSocket from parent module
use super::_ssl::{
    PySSLSocket, SSL3_RT_MAX_PACKET_SIZE, VERIFY_X509_PARTIAL_CHAIN, VERIFY_X509_STRICT,
};

// Import error types and helper functions from error module
use super::error::{
    PySSLCertVerificationError, PySSLError, PySSLWantWriteError, create_ssl_eof_error,
    create_ssl_syscall_error, create_ssl_want_read_error, create_ssl_want_write_error,
    create_ssl_zero_return_error,
};

// OpenSSL Constants:

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

pub(super) const X509_V_ERR_UNSPECIFIED: i32 = 1;
pub(super) const X509_V_ERR_UNABLE_TO_GET_CRL: i32 = 3;
pub(super) const X509_V_ERR_CERT_NOT_YET_VALID: i32 = 9;
pub(super) const X509_V_ERR_CERT_HAS_EXPIRED: i32 = 10;
pub(super) const X509_V_ERR_UNABLE_TO_GET_ISSUER_CERT_LOCALLY: i32 = 20;
pub(super) const X509_V_ERR_CERT_REVOKED: i32 = 23;
pub(super) const X509_V_ERR_INVALID_PURPOSE: i32 = 26;
pub(super) const X509_V_ERR_HOSTNAME_MISMATCH: i32 = 62;
pub(super) const X509_V_ERR_IP_ADDRESS_MISMATCH: i32 = 64;

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
    /// Non-TLS data received before handshake completed
    PreauthData,
    /// Certificate verification error
    CertVerification(rustls::CertificateError),
    /// I/O error
    Io(std::io::Error),
    /// Timeout error (socket.timeout)
    Timeout(String),
    /// Python exception (pass through directly)
    Py(PyBaseExceptionRef),
    /// Preserve the TLS error until the Python exception boundary.
    Rustls(rustls::Error),
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
    pub(super) fn from_rustls(err: rustls::Error) -> Self {
        Self::Rustls(err)
    }

    pub(super) fn is_eof(&self) -> bool {
        match self {
            Self::Rustls(error) => matches!(Self::map_rustls(error.clone()), Self::Eof),
            Self::Eof => true,
            _ => false,
        }
    }

    pub(super) fn is_zero_return(&self) -> bool {
        matches!(
            self,
            Self::ZeroReturn
                | Self::Rustls(rustls::Error::AlertReceived(
                    rustls::AlertDescription::CloseNotify
                ))
        )
    }

    fn map_rustls(err: rustls::Error) -> Self {
        match err {
            rustls::Error::InvalidCertificate(cert_err) => Self::CertVerification(cert_err),
            rustls::Error::AlertReceived(alert_desc) => {
                // Map TLS alerts to OpenSSL-compatible error codes
                // lib = 20 (ERR_LIB_SSL), reason = 1000 + alert_code
                match alert_desc {
                    rustls::AlertDescription::CloseNotify => {
                        // Special case: close_notify is handled as ZeroReturn
                        Self::ZeroReturn
                    }
                    _ => {
                        // All other alerts: convert to OpenSSL error code
                        // This includes InternalError (80 -> reason 1080)
                        Self::AlertReceived {
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
                Self::Eof
            }
            rustls::Error::PeerIncompatible(peer_err) => {
                // Check for specific incompatibility types
                use rustls::PeerIncompatible;
                match peer_err {
                    PeerIncompatible::NoCipherSuitesInCommon => {
                        // Maps to OpenSSL SSL_R_NO_SHARED_CIPHER (lib=20, reason=193)
                        Self::NoCipherSuites
                    }
                    _ => {
                        // Other protocol incompatibilities → SSLEOFError
                        Self::Eof
                    }
                }
            }
            _ => Self::Ssl(format!("{err}")),
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
    pub(super) fn into_py_err(self, vm: &VirtualMachine) -> PyBaseExceptionRef {
        match self {
            Self::Rustls(error) => Self::map_rustls(error).into_py_err(vm),
            Self::WantRead => create_ssl_want_read_error(vm).upcast(),
            Self::WantWrite => create_ssl_want_write_error(vm).upcast(),
            Self::Timeout(msg) => timeout_error_msg(vm, msg).upcast(),
            Self::Syscall(msg) => {
                // SSLSyscallError with errno=SSL_ERROR_SYSCALL (5)
                create_ssl_syscall_error(vm, msg).upcast()
            }
            Self::Ssl(msg) => vm
                .new_os_subtype_error(
                    PySSLError::class(&vm.ctx).to_owned(),
                    None,
                    format!("SSL error: {msg}"),
                )
                .upcast(),
            Self::ZeroReturn => create_ssl_zero_return_error(vm).upcast(),
            Self::Eof => create_ssl_eof_error(vm).upcast(),
            Self::PreauthData => {
                // Non-TLS data received before handshake
                Self::create_ssl_error_with_reason(
                    vm,
                    None,
                    "before TLS handshake with data",
                    "before TLS handshake with data",
                )
            }
            Self::CertVerification(cert_err) => {
                // Use the proper cert verification error creator
                create_ssl_cert_verification_error(vm, &cert_err).expect("unlikely to happen")
            }
            Self::Io(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => {
                create_ssl_eof_error(vm).upcast()
            }
            Self::Io(err) if err.raw_os_error().is_none() => vm
                .new_os_subtype_error(
                    PySSLError::class(&vm.ctx).to_owned(),
                    None,
                    format!("SSL error: {err}"),
                )
                .upcast(),
            Self::Io(err) => err.into_pyexception(vm),

            Self::Py(exc) => exc,
            Self::AlertReceived { lib, reason } => {
                Self::create_ssl_error_from_codes(vm, lib, reason)
            }
            Self::NoCipherSuites => {
                // OpenSSL error: lib=20 (ERR_LIB_SSL), reason=193 (SSL_R_NO_SHARED_CIPHER)
                Self::create_ssl_error_from_codes(vm, ERR_LIB_SSL, SSL_R_NO_SHARED_CIPHER)
            }
        }
    }
}

pub(super) type SslResult<T> = Result<T, SslError>;
/// Common protocol settings shared between client and server connections
#[derive(Debug)]
pub(super) struct ProtocolSettings {
    pub versions: &'static [&'static rustls::SupportedProtocolVersion],
    pub kx_groups: Option<Vec<&'static dyn rustls::crypto::SupportedKxGroup>>,
    pub cipher_suites: Option<Vec<rustls::SupportedCipherSuite>>,
    pub alpn_protocols: Vec<Vec<u8>>,
}

/// Options for creating a server TLS configuration
#[derive(Debug)]
pub(super) struct ServerConfigOptions {
    /// Common protocol settings (versions, ALPN, KX groups, cipher suites)
    pub protocol_settings: ProtocolSettings,
    /// Server certificate chain
    pub cert_chain: Vec<CertificateDer<'static>>,
    /// Server private key
    pub private_key: PrivateKeyDer<'static>,
    /// Root certificates for client verification (if required)
    pub root_store: Option<RootCertStore>,
    pub ca_certs_der: Vec<Vec<u8>>,
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
    pub ticketer: Option<Arc<dyn ProducesTickets>>,
}

/// Options for creating a client TLS configuration
#[derive(Debug)]
pub(super) struct ClientConfigOptions {
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
    cipher_suites: Option<Vec<SupportedCipherSuite>>,
    kx_groups: Option<Vec<&'static dyn SupportedKxGroup>>,
) -> Arc<CryptoProvider> {
    let default_provider = CryptoExt::get_provider();

    Arc::new(CryptoProvider {
        cipher_suites: cipher_suites.unwrap_or_else(|| default_provider.cipher_suites.clone()),
        kx_groups: kx_groups.unwrap_or_else(|| default_provider.kx_groups.clone()),
        signature_verification_algorithms: default_provider.signature_verification_algorithms,
        secure_random: default_provider.secure_random,
        key_provider: default_provider.key_provider,
    })
}

/// Create a server TLS configuration
///
/// This abstracts the complex rustls ServerConfig building logic,
/// matching SSL_CTX initialization for server sockets.
pub(super) fn create_server_config(
    options: ServerConfigOptions,
) -> Result<chain::ServerConfig, String> {
    // Create custom crypto provider using helper function
    let custom_provider = create_custom_crypto_provider(
        options.protocol_settings.cipher_suites.clone(),
        options.protocol_settings.kx_groups.clone(),
    );

    let chain_builder = Arc::new(VerifiedChainBuilder {
        purpose: if options.request_client_cert {
            Purpose::ClientAuth
        } else {
            Purpose::Unverified
        },
        roots: options
            .root_store
            .clone()
            .unwrap_or_else(RootCertStore::empty),
        root_der: options.ca_certs_der,
        crls: Vec::new(),
        only_end_entity_revocation: false,
        supported: custom_provider.signature_verification_algorithms,
        allow_trusted_leaf: false,
        verify_flags: 0,
    });

    // Step 1: Build the appropriate client cert verifier based on settings
    let client_cert_verifier: Option<Arc<dyn rustls::server::danger::ClientCertVerifier>> =
        if let Some(root_store) = options.root_store {
            if options.request_client_cert {
                // Client certificate verification required
                let base_verifier = WebPkiClientVerifier::builder_with_provider(
                    Arc::new(root_store),
                    custom_provider.clone(),
                )
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
    let builder = ServerConfig::builder_with_provider(custom_provider)
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

    Ok((Arc::new(config), chain_builder))
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
pub(super) fn create_client_config(
    options: ClientConfigOptions,
) -> Result<chain::ClientConfig, String> {
    // Create custom crypto provider using helper function
    let custom_provider = create_custom_crypto_provider(
        options.protocol_settings.cipher_suites.clone(),
        options.protocol_settings.kx_groups.clone(),
    );

    let chain_builder = Arc::new(VerifiedChainBuilder {
        purpose: if options.verify_server_cert {
            Purpose::ServerAuth
        } else {
            Purpose::Unverified
        },
        roots: options
            .root_store
            .clone()
            .unwrap_or_else(RootCertStore::empty),
        root_der: options.ca_certs_der.clone(),
        crls: options.crls.clone(),
        only_end_entity_revocation: options.verify_flags & X509_V_FLAG_CRL_CHECK != 0,
        supported: custom_provider.signature_verification_algorithms,
        allow_trusted_leaf: options.check_hostname
            || options.verify_flags & VERIFY_X509_PARTIAL_CHAIN != 0,
        verify_flags: options.verify_flags,
    });

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
                    root_store_arc,
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
                    root_store_arc,
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
    let builder = ClientConfig::builder_with_provider(custom_provider)
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

    Ok((Arc::new(config), chain_builder))
}

/// Helper function - check if error is BlockingIOError
pub(super) fn is_blocking_io_error(err: &Py<PyBaseException>, vm: &VirtualMachine) -> bool {
    err.fast_isinstance(vm.ctx.exceptions.blocking_io_error)
}

// Socket I/O Helper Functions

/// Send all bytes to socket, handling partial sends with blocking wait
///
/// Loops until all bytes are sent. For blocking sockets, this will wait
/// until all data is sent. For non-blocking sockets, returns WantWrite
/// if no progress can be made.
/// Optional deadline parameter allows respecting a read deadline during flush.
pub(super) fn send_all_bytes(
    socket: &PySSLSocket,
    buf: Vec<u8>,
    vm: &VirtualMachine,
    deadline: Option<std::time::Instant>,
) -> SslResult<()> {
    // Retain newly drained records before a fallible flush of earlier output.
    socket.pending_tls_output.lock().extend_from_slice(&buf);
    socket
        .flush_pending_tls_output(vm, deadline)
        .map_err(|error| {
            if error.fast_isinstance(PySSLWantWriteError::class(&vm.ctx)) {
                SslError::WantWrite
            } else if error.fast_isinstance(vm.ctx.exceptions.timeout_error) {
                SslError::Timeout("The write operation timed out".to_owned())
            } else {
                SslError::Py(error)
            }
        })
}

// Handshake Helper Functions

/// Write TLS handshake data to socket/BIO
///
/// Drains all pending TLS data from rustls and sends it to the peer.
/// Returns whether any progress was made.
fn handshake_write_loop(
    conn: &mut Connection,
    socket: &PySSLSocket,
    vm: &VirtualMachine,
) -> SslResult<bool> {
    let mut made_progress = false;

    // Flush any previously pending TLS data before generating new output
    // Must succeed before sending new data to maintain order
    socket
        .flush_pending_tls_output(vm, None)
        .map_err(SslError::Py)?;

    while conn.wants_write() {
        let mut buf = Vec::new();
        let written = conn
            .write_tls(&mut buf as &mut dyn std::io::Write)
            .map_err(SslError::Io)?;

        if written > 0 && !buf.is_empty() {
            // Send all bytes to socket, handling partial sends
            send_all_bytes(socket, buf, vm, None)?;
            made_progress = true;
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

/// Read at most one TLS record from the TCP socket.
///
/// May return incomplete data but never returns more when completes a
/// previously incomplete TLS record.
///
/// OpenSSL reads one TLS record at a time (no read-ahead by default).
/// Rustls, however, consumes all available TCP data when fed via read_tls().
/// If a close_notify or other control record arrives alongside application
/// data, the eager read drains the TCP buffer, leaving the control record in
/// rustls's internal buffer where select() cannot see it.  This causes
/// asyncore-based servers (which rely on select() for readability) to miss
/// the data and the peer times out.
///
/// Fix: peek at the TCP buffer to find the first complete TLS record boundary
/// and recv() only that many bytes.  Any remaining data stays in the kernel
/// buffer and remains visible to select().
pub(super) fn recv_at_most_one_tls_record(
    socket: &PySSLSocket,
    vm: &VirtualMachine,
) -> SslResult<PyObjectRef> {
    let bytes = socket.sock_recv_at_most_one_tls_record(vm).map_err(|e| {
        if is_blocking_io_error(&e, vm) {
            SslError::WantRead
        } else {
            SslError::Py(e)
        }
    })?;
    if bytes.is_empty() {
        Err(if socket.is_bio_mode() && !socket.transport_eof() {
            SslError::WantRead
        } else {
            SslError::Eof
        })
    } else {
        Ok(bytes.into())
    }
}

/// Read up to a single TLS record for post-handshake I/O while preserving the
/// SSL-vs-socket error precedence from the old sock_recv() path.
fn recv_at_most_one_tls_record_for_data(
    conn: &mut Connection,
    socket: &PySSLSocket,
    vm: &VirtualMachine,
) -> SslResult<PyObjectRef> {
    match recv_at_most_one_tls_record(socket, vm) {
        Ok(data) => Ok(data),
        Err(SslError::Eof) => {
            if let Err(rustls_err) = conn.process_new_packets() {
                return Err(SslError::from_rustls(rustls_err));
            }
            Ok(vm.ctx.new_bytes(vec![]).into())
        }
        Err(SslError::Py(e)) => {
            if let Err(rustls_err) = conn.process_new_packets() {
                return Err(SslError::from_rustls(rustls_err));
            }
            if is_connection_closed_error(&e, vm) {
                return Err(SslError::Eof);
            }
            Err(SslError::Py(e))
        }
        Err(e) => Err(e),
    }
}

fn handshake_read_data(
    conn: &mut Connection,
    socket: &PySSLSocket,
    vm: &VirtualMachine,
) -> SslResult<()> {
    if socket
        .sock_wait_for_io_impl(SockWaitKind::Read, vm)
        .map_err(SslError::Py)?
    {
        return Err(SslError::Timeout(
            "The handshake operation timed out".to_owned(),
        ));
    }
    let data = recv_at_most_one_tls_record(socket, vm)?;
    ssl_read_tls_records(conn, data, socket.is_bio_mode(), vm)
}

/// Try to read plaintext data from TLS connection buffer
///
/// Returns Ok(Some(n)) if n bytes were read, Ok(None) if would block,
/// or Err on real errors.
fn try_read_plaintext(conn: &mut Connection, buf: &mut [u8]) -> SslResult<Option<usize>> {
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
    conn: &mut Connection,
    socket: &PySSLSocket,
    vm: &VirtualMachine,
) -> SslResult<()> {
    loop {
        // Both transports drain writes first and feed complete/partial records
        // through the same path. An empty BIO naturally returns WantRead.
        handshake_write_loop(conn, socket, vm)?;
        if !conn.is_handshaking() {
            return Ok(());
        }
        if !conn.wants_read() {
            return Err(SslError::WantRead);
        }
        handshake_read_data(conn, socket, vm)?;
        if let Err(error) = conn.process_new_packets() {
            return Err(if matches!(error, rustls::Error::InvalidMessage(_)) {
                SslError::PreauthData
            } else {
                SslError::from_rustls(error)
            });
        }
    }
}

/// Equivalent to OpenSSL's SSL_read()
///
/// Reads application data from TLS connection.
/// Automatically handles TLS record I/O as needed.
///
/// = SSL_read_ex()
pub(super) fn ssl_read(
    conn: &mut Connection,
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

    // CRITICAL: Flush any pending TLS output before reading
    // This ensures data from previous write() calls is sent before we wait for response.
    // Without this, write() may leave data in pending_tls_output (if socket buffer was full),
    // and read() would timeout waiting for a response that the server never received.
    if !is_bio {
        socket
            .flush_pending_tls_output(vm, deadline)
            .map_err(SslError::Py)?;
    }

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
            if n == 0 {
                // EOF from TLS - close_notify received
                // Return ZeroReturn so Python raises SSLZeroReturnError
                return Err(SslError::ZeroReturn);
            }
            return Ok(n);
        }

        // No plaintext available and rustls doesn't want to read more TLS records
        if !needs_more_tls {
            // Check if connection needs to write data first (e.g., TLS key update, renegotiation)
            // This mirrors the handshake logic which checks both wants_read() and wants_write()
            if conn.wants_write() && !is_bio {
                // Check deadline BEFORE attempting flush
                if let Some(deadline) = deadline
                    && std::time::Instant::now() >= deadline
                {
                    return Err(SslError::Timeout(
                        "The read operation timed out".to_string(),
                    ));
                }

                // Flush pending TLS data before continuing
                // CRITICAL: Pass deadline so flush respects read timeout
                let tls_data = ssl_write_tls_records(conn)?;
                if !tls_data.is_empty() {
                    // Use best-effort send - don't fail READ just because WRITE couldn't complete
                    match send_all_bytes(socket, tls_data, vm, deadline) {
                        Ok(()) => {}
                        Err(SslError::WantWrite) => {
                            // Socket buffer full - acceptable during READ operation
                            // Pending data will be sent on next write/read call
                        }
                        Err(SslError::Timeout(_)) => {
                            // Timeout during flush is acceptable during READ
                            // Pending data stays buffered for next operation
                        }
                        Err(e) => return Err(e),
                    }
                }

                // Check deadline AFTER flush attempt
                if let Some(deadline) = deadline
                    && std::time::Instant::now() >= deadline
                {
                    return Err(SslError::Timeout(
                        "The read operation timed out".to_string(),
                    ));
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

            // For non-blocking sockets, return WantRead so caller can poll and retry.
            // For blocking sockets (or sockets with timeout), wait for more data.
            if !is_bio {
                let timeout = socket.get_socket_timeout(vm).map_err(SslError::Py)?;
                if let Some(t) = timeout
                    && t.is_zero()
                {
                    // Non-blocking socket: check if peer has closed before returning WantRead
                    // If close_notify was received, we should return ZeroReturn (EOF), not WantRead
                    // This is critical for asyncore-based applications that rely on recv() returning
                    // 0 or raising SSL_ERROR_ZERO_RETURN to detect connection close.
                    let io_state = conn.process_new_packets().map_err(SslError::from_rustls)?;
                    if io_state.peer_has_closed() {
                        return Err(SslError::ZeroReturn);
                    }
                    // Non-blocking socket: return immediately
                    return Err(SslError::WantRead);
                }
                // Blocking socket or socket with timeout: try to read more data from socket.
                // Even though rustls says it doesn't want to read, more TLS records may arrive.
                // Use single-record reading to avoid consuming close_notify alongside data.
                let data = recv_at_most_one_tls_record_for_data(conn, socket, vm)?;

                let bytes_read = data
                    .clone()
                    .try_into_value::<rustpython_vm::builtins::PyBytes>(vm)
                    .map_or(0, |b| b.as_bytes().len());

                if bytes_read == 0 {
                    // No more data available - check if this is clean shutdown or unexpected EOF
                    // If close_notify was already received, return ZeroReturn (clean closure)
                    // Otherwise, return Eof (unexpected EOF)
                    let io_state = conn.process_new_packets().map_err(SslError::from_rustls)?;
                    if io_state.peer_has_closed() {
                        return Err(SslError::ZeroReturn);
                    }
                    return Err(SslError::Eof);
                }

                // Feed data to rustls and process
                ssl_read_tls_records(conn, data, false, vm)?;
                conn.process_new_packets().map_err(SslError::from_rustls)?;

                // Continue loop to try reading plaintext
                continue;
            }

            return Err(SslError::WantRead);
        }

        // Read and process TLS records
        match ssl_ensure_data_available(conn, socket, vm) {
            Ok(_bytes_read) => {
                // Successfully read and processed TLS data
                // Continue loop to try reading plaintext
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

/// Equivalent to OpenSSL's SSL_write()
///
/// Writes application data to TLS connection.
/// Automatically handles TLS record I/O as needed.
///
/// = SSL_write_ex()
pub(super) fn ssl_write(
    conn: &mut Connection,
    data: &[u8],
    socket: &PySSLSocket,
    vm: &VirtualMachine,
) -> SslResult<usize> {
    if data.is_empty() {
        return Ok(0);
    }

    let is_bio = socket.is_bio_mode();

    // Get socket timeout and calculate deadline (= _PyDeadline_Init)
    let deadline = if !is_bio {
        match socket.get_socket_timeout(vm).map_err(SslError::Py)? {
            Some(timeout) if !timeout.is_zero() => Some(std::time::Instant::now() + timeout),
            _ => None,
        }
    } else {
        None
    };

    // Flush any pending TLS output before writing new data
    if !is_bio {
        socket
            .flush_pending_tls_output(vm, deadline)
            .map_err(SslError::Py)?;
    }

    // Check if we already have data buffered from a previous retry
    // (prevents duplicate writes when retrying after WantWrite/WantRead)
    let already_buffered = *socket.write_buffered_len.lock();

    // Only write plaintext if not already buffered
    // Track how much we wrote for partial write handling
    let mut bytes_written_to_rustls = 0usize;

    if already_buffered == 0 {
        // Write plaintext to rustls (= SSL_write_ex internal buffer write)
        bytes_written_to_rustls = {
            let mut writer = conn.writer();
            use std::io::Write;
            // Use write() instead of write_all() to support partial writes.
            // In BIO mode (asyncio), when the internal buffer is full,
            // we want to write as much as possible and return that count,
            // rather than failing completely.
            match writer.write(data) {
                Ok(0) if !data.is_empty() => {
                    // Buffer is full and nothing could be written.
                    // In BIO mode, return WantWrite so the caller can
                    // drain the outgoing BIO and retry.
                    if is_bio {
                        return Err(SslError::WantWrite);
                    }
                    return Err(SslError::Syscall("Write failed: buffer full".to_string()));
                }
                Ok(n) => n,
                Err(e) => {
                    if is_bio {
                        // In BIO mode, treat write errors as WantWrite
                        return Err(SslError::WantWrite);
                    }
                    return Err(SslError::Syscall(format!("Write failed: {e}")));
                }
            }
        };
        // Mark data as buffered (only the portion we actually wrote)
        *socket.write_buffered_len.lock() = bytes_written_to_rustls;
    } else if already_buffered != data.len() {
        // Caller is retrying with different data - this is a protocol error
        // Clear the buffer state and return an SSL error (bad write retry)
        *socket.write_buffered_len.lock() = 0;
        return Err(SslError::Ssl("bad write retry".to_string()));
    }
    // else: already_buffered == data.len(), this is a valid retry

    // Loop to send TLS records, handling WANT_READ/WANT_WRITE
    // Matches CPython's do-while loop on SSL_ERROR_WANT_READ/WANT_WRITE
    loop {
        // Check deadline
        if let Some(dl) = deadline
            && std::time::Instant::now() >= dl
        {
            return Err(SslError::Timeout(
                "The write operation timed out".to_string(),
            ));
        }

        // Check if rustls has TLS data to send
        if !conn.wants_write() {
            // All TLS data sent successfully
            break;
        }

        // Get TLS records from rustls
        let tls_data = ssl_write_tls_records(conn)?;
        if tls_data.is_empty() {
            break;
        }

        // Send TLS data to socket
        match send_all_bytes(socket, tls_data, vm, deadline) {
            Ok(()) => {
                // Successfully sent, continue loop to check for more data
            }
            Err(SslError::WantWrite) => {
                // Non-blocking socket would block - return WANT_WRITE
                // If we had a partial write to rustls, return partial success
                // instead of error to match OpenSSL partial-write semantics
                if bytes_written_to_rustls > 0 && bytes_written_to_rustls < data.len() {
                    *socket.write_buffered_len.lock() = 0;
                    return Ok(bytes_written_to_rustls);
                }
                // Keep write_buffered_len set so we don't re-buffer on retry
                return Err(SslError::WantWrite);
            }
            Err(SslError::WantRead) => {
                // Need to read before write can complete (e.g., renegotiation)
                if is_bio {
                    // If we had a partial write to rustls, return partial success
                    if bytes_written_to_rustls > 0 && bytes_written_to_rustls < data.len() {
                        *socket.write_buffered_len.lock() = 0;
                        return Ok(bytes_written_to_rustls);
                    }
                    // Keep write_buffered_len set so we don't re-buffer on retry
                    return Err(SslError::WantRead);
                }
                // For socket mode, try to read TLS data
                let recv_result = recv_at_most_one_tls_record_for_data(conn, socket, vm)?;
                ssl_read_tls_records(conn, recv_result, false, vm)?;
                conn.process_new_packets().map_err(SslError::from_rustls)?;
                // Continue loop
            }
            Err(e @ SslError::Timeout(_)) => {
                // If we had a partial write to rustls, return partial success
                if bytes_written_to_rustls > 0 && bytes_written_to_rustls < data.len() {
                    *socket.write_buffered_len.lock() = 0;
                    return Ok(bytes_written_to_rustls);
                }
                // Preserve buffered state so retry doesn't duplicate data
                // (send_all_bytes saved unsent TLS bytes to pending_tls_output)
                return Err(e);
            }
            Err(e) => {
                // Clear buffer state on error
                *socket.write_buffered_len.lock() = 0;
                return Err(e);
            }
        }
    }

    // Final flush to ensure all data is sent
    if !is_bio {
        socket
            .flush_pending_tls_output(vm, deadline)
            .map_err(SslError::Py)?;
    }

    // Determine how many bytes we actually wrote
    let actual_written = if bytes_written_to_rustls > 0 {
        // Fresh write: return what we wrote to rustls
        bytes_written_to_rustls
    } else if already_buffered > 0 {
        // Retry of previous write: return the full buffered amount
        already_buffered
    } else {
        data.len()
    };

    // Write completed successfully - clear buffer state
    *socket.write_buffered_len.lock() = 0;

    Ok(actual_written)
}

// Helper functions (private-ish, used by public SSL functions)

/// Write TLS records from rustls to socket
fn ssl_write_tls_records(conn: &mut Connection) -> SslResult<Vec<u8>> {
    let mut buf = Vec::new();
    let n = conn
        .write_tls(&mut buf as &mut dyn std::io::Write)
        .map_err(SslError::Io)?;

    if n > 0 { Ok(buf) } else { Ok(Vec::new()) }
}

/// Read TLS records from socket to rustls
pub(super) fn ssl_read_tls_records(
    conn: &mut Connection,
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
        }
        // In socket mode, empty recv() means TCP EOF (FIN received)
        // Need to distinguish:
        // 1. Clean shutdown: received TLS close_notify → return ZeroReturn (0 bytes)
        // 2. Unexpected EOF: no close_notify → return Eof (SSLEOFError)
        //
        // SSL_ERROR_ZERO_RETURN vs SSL_ERROR_EOF logic
        // CPython checks SSL_get_shutdown() & SSL_RECEIVED_SHUTDOWN
        //
        // Process any buffered TLS records (may contain close_notify)
        match conn.process_new_packets() {
            Ok(io_state) => {
                if io_state.peer_has_closed() {
                    // Received close_notify - normal SSL closure (SSL_ERROR_ZERO_RETURN)
                    return Err(SslError::ZeroReturn);
                }
                // No close_notify - ragged EOF (SSL_ERROR_EOF → SSLEOFError)
                // CPython raises SSLEOFError here, which SSLSocket.read() handles
                // based on suppress_ragged_eofs setting
                return Err(SslError::Eof);
            }
            Err(e) => return Err(SslError::from_rustls(e)),
        }
    }

    // Feed all received data to read_tls - loop to consume all data
    // read_tls may not consume all data in one call, and buffer may become full
    let mut offset = 0;
    while offset < bytes_data.len() {
        let remaining = &bytes_data[offset..];
        let mut cursor = std::io::Cursor::new(remaining);

        match conn.read_tls(&mut cursor) {
            Ok(read_bytes) => {
                if read_bytes == 0 {
                    // Buffer is full - process existing packets to make room
                    conn.process_new_packets().map_err(SslError::from_rustls)?;

                    // Try again - if we still can't consume, break
                    let mut retry_cursor = std::io::Cursor::new(remaining);
                    match conn.read_tls(&mut retry_cursor) {
                        Ok(0) => {
                            // Still can't consume - break to avoid infinite loop
                            break;
                        }
                        Ok(n) => {
                            offset += n;
                            if offset < bytes_data.len() {
                                conn.process_new_packets().map_err(SslError::from_rustls)?;
                            }
                        }
                        Err(e) => {
                            return Err(SslError::Io(e));
                        }
                    }
                } else {
                    offset += read_bytes;
                    if offset < bytes_data.len() {
                        conn.process_new_packets().map_err(SslError::from_rustls)?;
                    }
                }
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
    conn: &mut Connection,
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
                    .sock_wait_for_io_impl(SockWaitKind::Read, vm)
                    .map_err(SslError::Py)?;
                if timed_out {
                    // Socket not ready within timeout - raise socket.timeout
                    return Err(SslError::Timeout(
                        "The read operation timed out".to_string(),
                    ));
                }
            }
            // else: non-blocking socket (timeout=0) or blocking socket (timeout=None) - skip select
        }

        // Read one TLS record at a time for non-BIO sockets (matching
        // OpenSSL's default no-read-ahead behaviour).  This prevents
        // consuming a close_notify that arrives alongside application data,
        // keeping it in the kernel buffer where select() can detect it.
        let data = if !is_bio {
            recv_at_most_one_tls_record_for_data(conn, socket, vm)?
        } else {
            match socket.sock_recv(SSL3_RT_MAX_PACKET_SIZE, vm) {
                Ok(data) => data,
                Err(e) => {
                    if is_blocking_io_error(&e, vm) {
                        return Err(SslError::WantRead);
                    }
                    if let Err(rustls_err) = conn.process_new_packets() {
                        return Err(SslError::from_rustls(rustls_err));
                    }
                    if is_connection_closed_error(&e, vm) {
                        return Err(SslError::Eof);
                    }
                    return Err(SslError::Py(e));
                }
            }
        };

        // Get the size of received data
        let bytes_read = data
            .clone()
            .try_into_value::<rustpython_vm::builtins::PyBytes>(vm)
            .map_or(0, |b| b.as_bytes().len());

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
    pub(super) fn new(cert_keys: Vec<Arc<CertifiedKey>>) -> Self {
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

/// Convert curve name to rustls key exchange group
///
/// Maps OpenSSL curve names (e.g., "prime256v1", "secp384r1") to rustls KxGroups.
/// Returns an error if the curve is not supported by rustls.
pub(super) fn curve_name_to_kx_group(
    curve: &str,
) -> Result<Vec<&'static dyn SupportedKxGroup>, String> {
    super::cipher::kx_group_by_openssl_name(curve)
        .map(|group| vec![group])
        .ok_or_else(|| format!("unknown curve name '{curve}'"))
}
