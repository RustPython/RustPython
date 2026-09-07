//! Host rustls `_ssl` engine.
//!
//! TLS helpers live here because they share the host trust store, default
//! verify paths, and certificate files with the rest of `host_env`. The
//! engine reports plain Rust results so interpreter layers can provide their
//! own object and exception adapters.

pub mod bio;
pub mod cert;
pub mod chain;
pub mod cipher;
pub mod config;
pub mod constants;
pub mod handshake;
pub mod keylog;
pub mod msg;
pub mod oid;
pub mod providers;
pub mod verify;
pub mod x509;

pub use bio::{MemoryBio, MemoryBioError};
pub use cert::{DecodedCertificate, decode_certificate, is_ca_certificate};
pub use constants::*;
pub use providers::CryptoExt;
