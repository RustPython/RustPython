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
