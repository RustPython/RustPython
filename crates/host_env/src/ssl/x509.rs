//! X.509 verify codes used when mapping rustls certificate errors.

pub const X509_V_ERR_UNSPECIFIED: i32 = 1;
pub const X509_V_ERR_UNABLE_TO_GET_CRL: i32 = 3;
pub const X509_V_ERR_CERT_NOT_YET_VALID: i32 = 9;
pub const X509_V_ERR_CERT_HAS_EXPIRED: i32 = 10;
pub const X509_V_ERR_UNABLE_TO_GET_ISSUER_CERT_LOCALLY: i32 = 20;
pub const X509_V_ERR_CERT_REVOKED: i32 = 23;
pub const X509_V_ERR_INVALID_PURPOSE: i32 = 26;
pub const X509_V_ERR_HOSTNAME_MISMATCH: i32 = 62;
pub const X509_V_ERR_IP_ADDRESS_MISMATCH: i32 = 64;

/// Map a rustls certificate error to an X509 verify code and message.
#[must_use]
pub fn rustls_cert_error_to_verify_info(
    cert_err: &rustls::CertificateError,
) -> (i32, &'static str) {
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
