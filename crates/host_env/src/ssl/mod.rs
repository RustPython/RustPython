//! Shared `_ssl` surface.
//!
//! MemoryBIO, protocol constants, OID tables, hostname checks, and ALPN
//! parsing compile without rustls so wasm and `not(ssl)` hosts can implement
//! `_ssl` on the same types. The rustls engine stays behind the `ssl`
//! feature.

pub mod bio;
pub mod constants;
pub mod oid;
pub mod protocol;

#[cfg(feature = "ssl")]
pub mod cert;
#[cfg(feature = "ssl")]
pub mod chain;
#[cfg(feature = "ssl")]
pub mod cipher;
#[cfg(feature = "ssl")]
pub mod config;
#[cfg(feature = "ssl")]
pub mod connection;
#[cfg(feature = "ssl")]
pub mod error;
#[cfg(feature = "ssl")]
pub mod handshake;
#[cfg(feature = "ssl")]
pub mod keylog;
#[cfg(feature = "ssl")]
pub mod msg;
#[cfg(feature = "ssl")]
pub mod providers;
#[cfg(feature = "ssl")]
pub mod session;
#[cfg(feature = "ssl")]
pub mod verify;
#[cfg(feature = "ssl")]
pub mod x509;

pub use bio::{MemoryBio, MemoryBioError};
pub use constants::*;
pub use protocol::{AlpnError, HostnameError, parse_length_prefixed_alpn, validate_hostname};

#[cfg(feature = "ssl")]
pub use cert::{DecodedCertificate, decode_certificate, is_ca_certificate};
#[cfg(feature = "ssl")]
pub use connection::{
    RecordCursor, SSL3_RT_MAX_PACKET_SIZE, TLS_RECORD_HEADER_SIZE, TlsConnection,
};
#[cfg(feature = "ssl")]
pub use error::TlsError;
#[cfg(feature = "ssl")]
pub use protocol::rustls_versions;
#[cfg(feature = "ssl")]
pub use providers::CryptoExt;
#[cfg(feature = "ssl")]
pub use session::{
    CapturingClientSessionStore, ClientSessionKind, SESSION_CACHE_SIZE, SessionCache, SessionData,
};
