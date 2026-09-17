//! Shared `_ssl` surface.
//!
//! `ssl` is the rustls-free surface: MemoryBIO, constants, OID, ALPN, and
//! hostname checks. `ssl-rustls` compiles the rustls engine. Browser wasm
//! enables `ssl-rustls` together with rustls-pki-types `web` on that crate;
//! wasm-host enables `ssl` only.

pub mod bio;
pub mod constants;
pub mod oid;
pub mod protocol;

#[cfg(feature = "ssl-rustls")]
pub mod cert;
#[cfg(feature = "ssl-rustls")]
pub mod chain;
#[cfg(feature = "ssl-rustls")]
pub mod cipher;
#[cfg(feature = "ssl-rustls")]
pub mod config;
#[cfg(feature = "ssl-rustls")]
pub mod connection;
#[cfg(feature = "ssl-rustls")]
pub mod error;
#[cfg(feature = "ssl-rustls")]
pub mod handshake;
#[cfg(feature = "ssl-rustls")]
pub mod keylog;
#[cfg(feature = "ssl-rustls")]
pub mod msg;
#[cfg(feature = "ssl-rustls")]
pub mod providers;
#[cfg(feature = "ssl-rustls")]
pub mod session;
#[cfg(feature = "ssl-rustls")]
pub mod verify;
#[cfg(feature = "ssl-rustls")]
pub mod x509;

pub use bio::{MemoryBio, MemoryBioError};
pub use constants::*;
pub use protocol::{AlpnError, HostnameError, parse_length_prefixed_alpn, validate_hostname};

#[cfg(feature = "ssl-rustls")]
pub use cert::{DecodedCertificate, decode_certificate, is_ca_certificate};
#[cfg(feature = "ssl-rustls")]
pub use connection::{
    RecordCursor, SSL3_RT_MAX_PACKET_SIZE, TLS_RECORD_HEADER_SIZE, TlsConnection,
};
#[cfg(feature = "ssl-rustls")]
pub use error::TlsError;
#[cfg(feature = "ssl-rustls")]
pub use protocol::rustls_versions;
#[cfg(feature = "ssl-rustls")]
pub use providers::CryptoExt;
#[cfg(feature = "ssl-rustls")]
pub use session::{
    CapturingClientSessionStore, ClientSessionKind, SESSION_CACHE_SIZE, SessionCache, SessionData,
};
