//! Protocol, verification, and option values published by `_ssl`.

/// `_SSLContext` constructor protocol. Exclusive selector, not an `SSL_OP_*` mask.
#[repr(i32)]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SslVersion {
    Tls = 2,
    Tls1 = 3,
    Tls1_1 = 4,
    Tls1_2 = 5,
    Tls1_3 = 6,
    TlsClient = 0x10,
    TlsServer = 0x11,
}

pub const PROTOCOL_TLS: i32 = SslVersion::Tls as i32;
pub const PROTOCOL_TLS_CLIENT: i32 = SslVersion::TlsClient as i32;
pub const PROTOCOL_TLS_SERVER: i32 = SslVersion::TlsServer as i32;
pub const PROTOCOL_TLSV1: i32 = SslVersion::Tls1 as i32;
pub const PROTOCOL_TLSV1_1: i32 = SslVersion::Tls1_1 as i32;
pub const PROTOCOL_TLSV1_2: i32 = SslVersion::Tls1_2 as i32;
pub const PROTOCOL_TLSV1_3: i32 = SslVersion::Tls1_3 as i32;

/// `verify_mode`. Exclusive selector, not an `SSL_VERIFY_*` bitmask.
#[repr(i32)]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum CertRequirements {
    None = 0,
    Optional = 1,
    Required = 2,
}

pub const CERT_NONE: i32 = CertRequirements::None as i32;
pub const CERT_OPTIONAL: i32 = CertRequirements::Optional as i32;
pub const CERT_REQUIRED: i32 = CertRequirements::Required as i32;

/// TLS record version (`SSL3_VERSION` / `TLS1_*_VERSION` in `prov_ssl.h`).
#[repr(i32)]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ProtoVersion {
    MinSupported = -2,
    Ssl3 = 0x0300,
    Tls1 = 0x0301,
    Tls1_1 = 0x0302,
    Tls1_2 = 0x0303,
    Tls1_3 = 0x0304,
    MaxSupported = -1,
}

pub const PROTO_TLSV1_2: i32 = ProtoVersion::Tls1_2 as i32;
pub const PROTO_TLSV1_3: i32 = ProtoVersion::Tls1_3 as i32;

/// `X509_V_FLAG_*` bitmasks from `x509_vfy.h`.
pub const VERIFY_DEFAULT: i32 = 0;
pub const VERIFY_CRL_CHECK_LEAF: i32 = 4;
pub const VERIFY_CRL_CHECK_CHAIN: i32 = 12;
pub const VERIFY_X509_STRICT: i32 = 32;
pub const VERIFY_ALLOW_PROXY_CERTS: i32 = 64;
pub const VERIFY_X509_TRUSTED_FIRST: i32 = 32768;
pub const VERIFY_X509_PARTIAL_CHAIN: i32 = 0x80000;

/// `X509_CHECK_FLAG_NEVER_CHECK_SUBJECT`. When set, hostname checks
/// use SAN only and never fall back to the certificate Common Name.
pub const HOSTFLAG_NEVER_CHECK_SUBJECT: i32 = 0x20;

/// `SSL_OP_NO_TLSv1_2` / `SSL_OP_NO_TLSv1_3` from `ssl.h`.
pub const OP_NO_TLSV1_2: i32 = 0x0800_0000;
pub const OP_NO_TLSV1_3: i32 = 0x2000_0000;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_and_cert_discriminants_match_published_consts() {
        assert_eq!(PROTOCOL_TLS, 2);
        assert_eq!(PROTOCOL_TLSV1, 3);
        assert_eq!(PROTOCOL_TLSV1_1, 4);
        assert_eq!(PROTOCOL_TLSV1_2, 5);
        assert_eq!(PROTOCOL_TLSV1_3, 6);
        assert_eq!(PROTOCOL_TLS_CLIENT, 0x10);
        assert_eq!(PROTOCOL_TLS_SERVER, 0x11);
        assert_eq!(CERT_NONE, 0);
        assert_eq!(CERT_OPTIONAL, 1);
        assert_eq!(CERT_REQUIRED, 2);
        assert_eq!(PROTO_TLSV1_2, 0x0303);
        assert_eq!(PROTO_TLSV1_3, 0x0304);
        assert_eq!(ProtoVersion::Ssl3 as i32, 0x0300);
        assert_eq!(ProtoVersion::Tls1 as i32, 0x0301);
        assert_eq!(ProtoVersion::Tls1_1 as i32, 0x0302);
    }
}
