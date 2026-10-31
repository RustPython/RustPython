//! rustls errors mapped to the `_ssl` error space, without Python exceptions.

use rustls::CertificateError;

const ERR_LIB_SSL: i32 = 20;

/// TLS engine error before the interpreter builds an exception.
#[derive(Debug)]
pub enum TlsError {
    WantRead,
    WantWrite,
    Syscall(String),
    Ssl(String),
    ZeroReturn,
    Eof,
    PreauthData,
    CertVerification(CertificateError),
    Io(std::io::Error),
    Timeout(String),
    AlertReceived { lib: i32, reason: i32 },
    NoCipherSuites,
}

impl TlsError {
    fn alert_to_openssl_reason(alert: rustls::AlertDescription) -> i32 {
        1000 + i32::from(u8::from(alert))
    }

    /// Map a rustls error into the `_ssl` error space.
    #[must_use]
    pub fn from_rustls(err: rustls::Error) -> Self {
        match err {
            rustls::Error::InvalidCertificate(cert_err) => Self::CertVerification(cert_err),
            rustls::Error::AlertReceived(alert_desc) => match alert_desc {
                rustls::AlertDescription::CloseNotify => Self::ZeroReturn,
                _ => Self::AlertReceived {
                    lib: ERR_LIB_SSL,
                    reason: Self::alert_to_openssl_reason(alert_desc),
                },
            },
            rustls::Error::InvalidMessage(_) => Self::Eof,
            rustls::Error::PeerIncompatible(peer_err) => {
                use rustls::PeerIncompatible;
                match peer_err {
                    PeerIncompatible::NoCipherSuitesInCommon => Self::NoCipherSuites,
                    PeerIncompatible::NoSignatureSchemesInCommon => {
                        Self::Ssl("no signature schemes in common".to_owned())
                    }
                    _ => Self::Eof,
                }
            }
            other => Self::Ssl(format!("{other}")),
        }
    }

    #[must_use]
    pub fn is_eof(&self) -> bool {
        matches!(self, Self::Eof)
    }

    #[must_use]
    pub fn is_zero_return(&self) -> bool {
        matches!(self, Self::ZeroReturn)
    }
}

impl From<std::io::Error> for TlsError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_incompatibility_is_not_transport_eof() {
        let error = TlsError::from_rustls(rustls::Error::PeerIncompatible(
            rustls::PeerIncompatible::NoSignatureSchemesInCommon,
        ));
        assert!(!error.is_eof());
        assert!(matches!(error, TlsError::Ssl(_)));
    }

    #[test]
    fn unrelated_negotiation_and_transport_errors_keep_their_classification() {
        assert!(matches!(
            TlsError::from_rustls(rustls::Error::PeerIncompatible(
                rustls::PeerIncompatible::NoCipherSuitesInCommon,
            )),
            TlsError::NoCipherSuites
        ));
        assert!(matches!(
            TlsError::from_rustls(rustls::Error::PeerIncompatible(
                rustls::PeerIncompatible::Tls12NotOffered,
            )),
            TlsError::Eof
        ));
        let os_error = std::io::Error::from_raw_os_error(10053);
        match TlsError::from(os_error) {
            TlsError::Io(error) => assert_eq!(error.raw_os_error(), Some(10053)),
            error => panic!("OS error was reclassified: {error:?}"),
        }
    }
}
