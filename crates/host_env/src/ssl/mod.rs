//! Shared `_ssl` surface.
//!
//! The `ssl` feature compiles this whole module. MemoryBIO, constants, OID,
//! and ALPN do not use rustls. The rustls engine is compiled on native and
//! WASI; `wasm32-unknown-unknown` has no `UnixTime::now` / native crypto.

pub mod bio;
pub mod constants;
pub mod oid;
pub mod protocol;

#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
pub mod cert;
#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
pub mod chain;
#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
pub mod cipher;
#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
pub mod config;
#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
pub mod connection;
#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
pub mod error;
#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
pub mod handshake;
#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
pub mod keylog;
#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
pub mod msg;
#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
pub mod providers;
#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
pub mod session;
#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
pub mod verify;
#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
pub mod x509;

pub use bio::{MemoryBio, MemoryBioError};
pub use constants::*;
pub use protocol::{AlpnError, HostnameError, parse_length_prefixed_alpn, validate_hostname};

#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
pub use cert::{DecodedCertificate, decode_certificate, is_ca_certificate};
#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
pub use connection::{
    RecordCursor, SSL3_RT_MAX_PACKET_SIZE, TLS_RECORD_HEADER_SIZE, TlsConnection,
};
#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
pub use error::TlsError;
#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
pub use protocol::rustls_versions;
#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
pub use providers::CryptoExt;
#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
pub use session::{
    CapturingClientSessionStore, ClientSessionKind, SESSION_CACHE_SIZE, SessionCache, SessionData,
};
