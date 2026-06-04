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

use crate::socket::timeout_error_msg;
use crate::vm::VirtualMachine;
use rustpython_vm::builtins::PyBaseExceptionRef;
use rustpython_vm::convert::IntoPyException;
use rustpython_vm::{AsObject, PyPayload, PyResult};

// Import error types and helper functions from error module
use super::error::{
    PySSLCertVerificationError, PySSLError, create_ssl_eof_error, create_ssl_want_read_error,
    create_ssl_want_write_error, create_ssl_zero_return_error,
};

// OpenSSL Constants:

// OpenSSL error library codes (include/openssl/err.h)
// #define ERR_LIB_SSL 20
const ERR_LIB_SSL: i32 = 20;

// OpenSSL SSL error reason codes (include/openssl/sslerr.h)
const SSL_R_NO_SUITABLE_KEY_SHARE: i32 = 101;
const SSL_R_NO_SUITABLE_SIGNATURE_ALGORITHM: i32 = 118;
const SSL_R_NO_SHARED_CIPHER: i32 = 193;
const SSL_R_NO_APPLICATION_PROTOCOL: i32 = 235;
const SSL_R_UNSUPPORTED_PROTOCOL: i32 = 258;
const SSL_R_NO_SUITABLE_GROUPS: i32 = 295;

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
    /// SSL_ERROR_SSL
    Ssl(String),
    /// PEM parser error
    PemLib(String),
    /// DER parser error
    FailedToReadDer(String),
    /// Text cadata did not contain a certificate PEM block
    CadataNoStartLine,
    /// Binary cadata did not contain a DER certificate
    CadataNotEnoughData,
    /// SSL_ERROR_ZERO_RETURN (clean closure with close_notify)
    ZeroReturn,
    /// Unexpected EOF without close_notify (protocol violation)
    Eof,
    /// rustls error
    Rustls(rustls::Error),
    /// I/O error
    Io(std::io::Error),
    /// Timeout error (socket.timeout)
    #[expect(dead_code, reason = "TODO: Implement timeouts")]
    Timeout(String),
    /// Python exception (pass through directly)
    Py(PyBaseExceptionRef),
}

impl SslError {
    /// Convert TLS alert code to OpenSSL error reason code
    /// OpenSSL uses reason = 1000 + alert_code for TLS alerts
    fn alert_to_openssl_reason(alert: rustls::AlertDescription) -> i32 {
        // AlertDescription can be converted to u8 via as u8 cast
        1000 + (u8::from(alert) as i32)
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
        let message = format!(
            "[SSL: {reason_str}] {}",
            reason_str.to_ascii_lowercase().replace('_', " ")
        );
        Self::create_ssl_error_with_reason(vm, Some(lib_str), reason_str, message)
    }

    fn create_plain_ssl_error(vm: &VirtualMachine, msg: impl Into<String>) -> PyBaseExceptionRef {
        vm.new_os_subtype_error(
            PySSLError::class(&vm.ctx).to_owned(),
            None,
            format!("SSL error: {}", msg.into()),
        )
        .upcast()
    }

    fn create_pem_ssl_error(
        vm: &VirtualMachine,
        msg: impl Into<String>,
    ) -> PyResult<PyBaseExceptionRef> {
        let msg = msg.into();
        let exc = vm.new_os_subtype_error(
            PySSLError::class(&vm.ctx).to_owned(),
            None,
            format!("SSL error: {msg}"),
        );
        exc.as_object()
            .set_attr("library", vm.ctx.new_str("PEM").as_object().to_owned(), vm)?;
        exc.as_object()
            .set_attr("reason", vm.ctx.new_str(msg).as_object().to_owned(), vm)?;
        Ok(exc.upcast())
    }

    /// Convert to Python exception
    pub(super) fn into_py_err(self, vm: &VirtualMachine) -> PyBaseExceptionRef {
        match self {
            Self::WantRead => create_ssl_want_read_error(vm).upcast(),
            Self::WantWrite => create_ssl_want_write_error(vm).upcast(),
            Self::Timeout(msg) => timeout_error_msg(vm, msg).upcast(),
            Self::Ssl(msg) => Self::create_plain_ssl_error(vm, msg),
            Self::Rustls(err) => match err {
                rustls::Error::InvalidCertificate(cert_err) => {
                    create_ssl_cert_verification_error(vm, &cert_err).expect("unlikely to happen")
                }
                rustls::Error::AlertReceived(rustls::AlertDescription::CloseNotify) => {
                    create_ssl_zero_return_error(vm).upcast()
                }
                rustls::Error::AlertReceived(alert_desc) => Self::create_ssl_error_from_codes(
                    vm,
                    ERR_LIB_SSL,
                    Self::alert_to_openssl_reason(alert_desc),
                ),
                rustls::Error::PeerIncompatible(peer_err) => {
                    use rustls::PeerIncompatible;
                    let reason = match peer_err {
                        PeerIncompatible::NoCipherSuitesInCommon => SSL_R_NO_SHARED_CIPHER,
                        PeerIncompatible::NoKxGroupsInCommon
                        | PeerIncompatible::NoEcPointFormatsInCommon
                        | PeerIncompatible::EcPointsExtensionRequired
                        | PeerIncompatible::NamedGroupsExtensionRequired
                        | PeerIncompatible::UncompressedEcPointsRequired => {
                            SSL_R_NO_SUITABLE_GROUPS
                        }
                        PeerIncompatible::KeyShareExtensionRequired => SSL_R_NO_SUITABLE_KEY_SHARE,
                        PeerIncompatible::NoCertificateRequestSignatureSchemesInCommon
                        | PeerIncompatible::NoSignatureSchemesInCommon
                        | PeerIncompatible::SignatureAlgorithmsExtensionRequired => {
                            SSL_R_NO_SUITABLE_SIGNATURE_ALGORITHM
                        }
                        PeerIncompatible::ServerDoesNotSupportTls12Or13
                        | PeerIncompatible::ServerTlsVersionIsDisabledByOurConfig
                        | PeerIncompatible::SupportedVersionsExtensionRequired
                        | PeerIncompatible::Tls12NotOffered
                        | PeerIncompatible::Tls12NotOfferedOrEnabled
                        | PeerIncompatible::Tls13RequiredForQuic => SSL_R_UNSUPPORTED_PROTOCOL,
                        _ => {
                            return Self::create_plain_ssl_error(
                                vm,
                                format!("peer is incompatible: {peer_err:?}"),
                            );
                        }
                    };
                    Self::create_ssl_error_from_codes(vm, ERR_LIB_SSL, reason)
                }
                rustls::Error::NoApplicationProtocol => Self::create_ssl_error_from_codes(
                    vm,
                    ERR_LIB_SSL,
                    SSL_R_NO_APPLICATION_PROTOCOL,
                ),
                _ => Self::create_plain_ssl_error(vm, err.to_string()),
            },
            Self::PemLib(msg) => Self::create_pem_ssl_error(vm, format!("PEM lib: {msg}"))
                .expect("unlikely to happen"),
            Self::FailedToReadDer(msg) => {
                Self::create_plain_ssl_error(vm, format!("Failed to read DER: {msg}"))
            }
            Self::CadataNoStartLine => Self::create_plain_ssl_error(
                vm,
                "no start line: cadata does not contain a certificate",
            ),
            Self::CadataNotEnoughData => Self::create_plain_ssl_error(
                vm,
                "not enough data: cadata does not contain a certificate",
            ),
            Self::ZeroReturn => create_ssl_zero_return_error(vm).upcast(),
            Self::Eof => create_ssl_eof_error(vm).upcast(),
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
        }
    }
}

pub(super) type SslResult<T> = Result<T, SslError>;
