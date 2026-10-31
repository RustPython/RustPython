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

pub const PROTO_SSL3: i32 = ProtoVersion::Ssl3 as i32;
pub const PROTO_TLSV1: i32 = ProtoVersion::Tls1 as i32;
pub const PROTO_TLSV1_1: i32 = ProtoVersion::Tls1_1 as i32;
pub const PROTO_TLSV1_2: i32 = ProtoVersion::Tls1_2 as i32;
pub const PROTO_TLSV1_3: i32 = ProtoVersion::Tls1_3 as i32;
pub const PROTO_MINIMUM_SUPPORTED: i32 = ProtoVersion::MinSupported as i32;
pub const PROTO_MAXIMUM_SUPPORTED: i32 = ProtoVersion::MaxSupported as i32;

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

/// `SSL_OP_*` from `ssl.h`. Values the `_ssl` module publishes.
#[allow(non_upper_case_globals)]
pub const OP_NO_SSLv2: i32 = 0;
#[allow(non_upper_case_globals)]
pub const OP_NO_SSLv3: i32 = 0x0200_0000;
pub const OP_NO_TLSV1: i32 = 0x0400_0000;
pub const OP_NO_TLSV1_1: i32 = 0x1000_0000;
/// `SSL_OP_NO_TLSv1_2` / `SSL_OP_NO_TLSv1_3` from `ssl.h`.
pub const OP_NO_TLSV1_2: i32 = 0x0800_0000;
pub const OP_NO_TLSV1_3: i32 = 0x2000_0000;
pub const OP_NO_COMPRESSION: i32 = 0x0002_0000;
pub const OP_CIPHER_SERVER_PREFERENCE: i32 = 0x0040_0000;
pub const OP_SINGLE_DH_USE: i32 = 0;
pub const OP_SINGLE_ECDH_USE: i32 = 0;
pub const OP_NO_TICKET: i32 = 0x0000_4000;
pub const OP_LEGACY_SERVER_CONNECT: i32 = 0x0000_0004;
pub const OP_NO_RENEGOTIATION: i32 = 0x4000_0000;
pub const OP_IGNORE_UNEXPECTED_EOF: i32 = 0x0000_0080;
pub const OP_ENABLE_MIDDLEBOX_COMPAT: i32 = 0x0010_0000;
pub const OP_ALL: i32 = 0x0000_0BFB;

pub const SSL3_RT_CHANGE_CIPHER_SPEC: i32 = 20;
pub const SSL3_RT_ALERT: i32 = 21;
pub const SSL3_RT_HANDSHAKE: i32 = 22;
pub const SSL3_RT_APPLICATION_DATA: i32 = 23;
pub const SSL3_RT_HEADER: i32 = 256;
pub const SSL3_MT_CHANGE_CIPHER_SPEC: i32 = 0x0101;

/// `SSL_ERROR_*` from `ssl.h`, the values `_ssl` publishes.
pub const SSL_ERROR_NONE: i32 = 0;
pub const SSL_ERROR_SSL: i32 = 1;
pub const SSL_ERROR_WANT_READ: i32 = 2;
pub const SSL_ERROR_WANT_WRITE: i32 = 3;
pub const SSL_ERROR_WANT_X509_LOOKUP: i32 = 4;
pub const SSL_ERROR_SYSCALL: i32 = 5;
pub const SSL_ERROR_ZERO_RETURN: i32 = 6;
pub const SSL_ERROR_WANT_CONNECT: i32 = 7;
pub const SSL_ERROR_EOF: i32 = 8;
pub const SSL_ERROR_INVALID_ERROR_CODE: i32 = 10;

pub const TLS_ERROR_SSL: i32 = SSL_ERROR_SSL;
pub const TLS_ERROR_WANT_READ: i32 = SSL_ERROR_WANT_READ;
pub const TLS_ERROR_WANT_WRITE: i32 = SSL_ERROR_WANT_WRITE;
pub const TLS_ERROR_ZERO_RETURN: i32 = SSL_ERROR_ZERO_RETURN;
pub const TLS_ERROR_EOF: i32 = SSL_ERROR_EOF;
pub const TLS_ERROR_NO_MEMORY: i32 = 9;
pub const TLS_ERROR_CERT_VERIFY_BASE: i32 = 1_000;

/// `PEM_TYPE_*` / `d2i` encoding selectors `_ssl` publishes.
pub const ENCODING_PEM: i32 = 1;
pub const ENCODING_DER: i32 = 2;
pub const ENCODING_PEM_AUX: i32 = 0x101;

/// TLS alert descriptions (`SSL_AD_*` / `_TLSAlertType`).
pub const ALERT_DESCRIPTION_CLOSE_NOTIFY: i32 = 0;
pub const ALERT_DESCRIPTION_UNEXPECTED_MESSAGE: i32 = 10;
pub const ALERT_DESCRIPTION_BAD_RECORD_MAC: i32 = 20;
pub const ALERT_DESCRIPTION_DECRYPTION_FAILED: i32 = 21;
pub const ALERT_DESCRIPTION_RECORD_OVERFLOW: i32 = 22;
pub const ALERT_DESCRIPTION_DECOMPRESSION_FAILURE: i32 = 30;
pub const ALERT_DESCRIPTION_HANDSHAKE_FAILURE: i32 = 40;
pub const ALERT_DESCRIPTION_NO_CERTIFICATE: i32 = 41;
pub const ALERT_DESCRIPTION_BAD_CERTIFICATE: i32 = 42;
pub const ALERT_DESCRIPTION_UNSUPPORTED_CERTIFICATE: i32 = 43;
pub const ALERT_DESCRIPTION_CERTIFICATE_REVOKED: i32 = 44;
pub const ALERT_DESCRIPTION_CERTIFICATE_EXPIRED: i32 = 45;
pub const ALERT_DESCRIPTION_CERTIFICATE_UNKNOWN: i32 = 46;
pub const ALERT_DESCRIPTION_ILLEGAL_PARAMETER: i32 = 47;
pub const ALERT_DESCRIPTION_UNKNOWN_CA: i32 = 48;
pub const ALERT_DESCRIPTION_ACCESS_DENIED: i32 = 49;
pub const ALERT_DESCRIPTION_DECODE_ERROR: i32 = 50;
pub const ALERT_DESCRIPTION_DECRYPT_ERROR: i32 = 51;
pub const ALERT_DESCRIPTION_EXPORT_RESTRICTION: i32 = 60;
pub const ALERT_DESCRIPTION_PROTOCOL_VERSION: i32 = 70;
pub const ALERT_DESCRIPTION_INSUFFICIENT_SECURITY: i32 = 71;
pub const ALERT_DESCRIPTION_INTERNAL_ERROR: i32 = 80;
pub const ALERT_DESCRIPTION_INAPPROPRIATE_FALLBACK: i32 = 86;
pub const ALERT_DESCRIPTION_USER_CANCELLED: i32 = 90;
pub const ALERT_DESCRIPTION_NO_RENEGOTIATION: i32 = 100;
pub const ALERT_DESCRIPTION_MISSING_EXTENSION: i32 = 109;
pub const ALERT_DESCRIPTION_UNSUPPORTED_EXTENSION: i32 = 110;
pub const ALERT_DESCRIPTION_CERTIFICATE_UNOBTAINABLE: i32 = 111;
pub const ALERT_DESCRIPTION_UNRECOGNIZED_NAME: i32 = 112;
pub const ALERT_DESCRIPTION_BAD_CERTIFICATE_STATUS_RESPONSE: i32 = 113;
pub const ALERT_DESCRIPTION_BAD_CERTIFICATE_HASH_VALUE: i32 = 114;
pub const ALERT_DESCRIPTION_UNKNOWN_PSK_IDENTITY: i32 = 115;
pub const ALERT_DESCRIPTION_CERTIFICATE_REQUIRED: i32 = 116;
pub const ALERT_DESCRIPTION_NO_APPLICATION_PROTOCOL: i32 = 120;

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
        assert_eq!(PROTO_SSL3, 0x0300);
        assert_eq!(PROTO_TLSV1, 0x0301);
        assert_eq!(PROTO_TLSV1_1, 0x0302);
        assert_eq!(PROTO_TLSV1_2, 0x0303);
        assert_eq!(PROTO_TLSV1_3, 0x0304);
        assert_eq!(PROTO_MINIMUM_SUPPORTED, -2);
        assert_eq!(PROTO_MAXIMUM_SUPPORTED, -1);
        assert_eq!(ProtoVersion::Ssl3 as i32, 0x0300);
        assert_eq!(ProtoVersion::Tls1 as i32, 0x0301);
        assert_eq!(ProtoVersion::Tls1_1 as i32, 0x0302);
        assert_eq!(SSL_ERROR_SSL, 1);
        assert_eq!(TLS_ERROR_SSL, SSL_ERROR_SSL);
        assert_eq!(ENCODING_PEM, 1);
        assert_eq!(ENCODING_DER, 2);
        assert_eq!(ALERT_DESCRIPTION_CLOSE_NOTIFY, 0);
        assert_eq!(ALERT_DESCRIPTION_NO_APPLICATION_PROTOCOL, 120);
    }
}
