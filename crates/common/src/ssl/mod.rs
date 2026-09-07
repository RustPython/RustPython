//! VM-independent rustls `_ssl` engine.
//!
//! The engine owns TLS configuration helpers, certificate decoding, cipher
//! strings, OID tables, and in-memory BIO state, and reports plain Rust
//! results so interpreter and embedding layers can provide their own object
//! and exception adapters.

pub mod bio;
pub mod cert;
pub mod chain;
pub mod cipher;
pub mod constants;
pub mod msg;
pub mod oid;
pub mod providers;

pub use bio::{MemoryBio, MemoryBioError};
pub use cert::{DecodedCertificate, decode_certificate, is_ca_certificate};
pub use constants::*;
pub use providers::CryptoExt;
