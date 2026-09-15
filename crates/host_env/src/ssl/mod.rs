//! Shared `_ssl` surface.
//!
//! The `ssl` feature compiles this whole module. MemoryBIO, constants, OID,
//! and ALPN do not use rustls. The rustls engine compiles wherever `rustls`
//! is available, including browser wasm via rustls-rustcrypto.

pub mod bio;
pub mod cert;
pub mod chain;
pub mod cipher;
pub mod config;
pub mod connection;
pub mod constants;
pub mod error;
pub mod handshake;
pub mod keylog;
pub mod msg;
pub mod oid;
pub mod protocol;
pub mod providers;
pub mod session;
pub mod verify;
pub mod x509;

pub use bio::{MemoryBio, MemoryBioError};
pub use cert::{DecodedCertificate, decode_certificate, is_ca_certificate};
pub use connection::{
    RecordCursor, SSL3_RT_MAX_PACKET_SIZE, TLS_RECORD_HEADER_SIZE, TlsConnection,
};
pub use constants::*;
pub use error::TlsError;
pub use protocol::{
    AlpnError, HostnameError, parse_length_prefixed_alpn, rustls_versions, validate_hostname,
};
pub use providers::CryptoExt;
pub use session::{
    CapturingClientSessionStore, ClientSessionKind, SESSION_CACHE_SIZE, SessionCache, SessionData,
};
