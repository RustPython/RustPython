//! Protocol, verification, and option values published by `_ssl`.

pub const PROTOCOL_TLS: i32 = 2;
pub const PROTOCOL_TLS_CLIENT: i32 = 16;
pub const PROTOCOL_TLS_SERVER: i32 = 17;
pub const PROTOCOL_TLSV1: i32 = 3;
pub const PROTOCOL_TLSV1_1: i32 = 4;
pub const PROTOCOL_TLSV1_2: i32 = 5;
pub const PROTOCOL_TLSV1_3: i32 = 6;

pub const CERT_NONE: i32 = 0;
pub const CERT_OPTIONAL: i32 = 1;
pub const CERT_REQUIRED: i32 = 2;

pub const VERIFY_DEFAULT: i32 = 0;
pub const VERIFY_CRL_CHECK_LEAF: i32 = 4;
pub const VERIFY_CRL_CHECK_CHAIN: i32 = 12;
pub const VERIFY_X509_STRICT: i32 = 32;
pub const VERIFY_ALLOW_PROXY_CERTS: i32 = 64;
pub const VERIFY_X509_TRUSTED_FIRST: i32 = 32768;
pub const VERIFY_X509_PARTIAL_CHAIN: i32 = 0x80000;
