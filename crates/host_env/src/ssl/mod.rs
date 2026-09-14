//! Shared `_ssl` surface.
//!
//! The `ssl` feature compiles this whole module. MemoryBIO, constants, OID,
//! and ALPN do not use rustls. The rustls engine is native-only because
//! rustls cannot build for `wasm32-unknown-unknown`.

pub mod bio;
pub mod constants;
pub mod oid;
pub mod protocol;

#[cfg(not(target_arch = "wasm32"))]
pub mod cert;
#[cfg(not(target_arch = "wasm32"))]
pub mod chain;
#[cfg(not(target_arch = "wasm32"))]
pub mod cipher;
#[cfg(not(target_arch = "wasm32"))]
pub mod config;
#[cfg(not(target_arch = "wasm32"))]
pub mod connection;
#[cfg(not(target_arch = "wasm32"))]
pub mod error;
#[cfg(not(target_arch = "wasm32"))]
pub mod handshake;
#[cfg(not(target_arch = "wasm32"))]
pub mod keylog;
#[cfg(not(target_arch = "wasm32"))]
pub mod msg;
#[cfg(not(target_arch = "wasm32"))]
pub mod providers;
#[cfg(not(target_arch = "wasm32"))]
pub mod session;
#[cfg(not(target_arch = "wasm32"))]
pub mod verify;
#[cfg(not(target_arch = "wasm32"))]
pub mod x509;

pub use bio::{MemoryBio, MemoryBioError};
pub use constants::*;
pub use protocol::{AlpnError, HostnameError, parse_length_prefixed_alpn, validate_hostname};

#[cfg(not(target_arch = "wasm32"))]
pub use cert::{DecodedCertificate, decode_certificate, is_ca_certificate};
#[cfg(not(target_arch = "wasm32"))]
pub use connection::{
    RecordCursor, SSL3_RT_MAX_PACKET_SIZE, TLS_RECORD_HEADER_SIZE, TlsConnection,
};
#[cfg(not(target_arch = "wasm32"))]
pub use error::TlsError;
#[cfg(not(target_arch = "wasm32"))]
pub use protocol::rustls_versions;
#[cfg(not(target_arch = "wasm32"))]
pub use providers::CryptoExt;
#[cfg(not(target_arch = "wasm32"))]
pub use session::{
    CapturingClientSessionStore, ClientSessionKind, SESSION_CACHE_SIZE, SessionCache, SessionData,
};
