//! Utilities for user-settable cryptography providers.
//!
//! This has two main moving parts: [`CryptoProvider`] and [`CryptoExt`]. [`CryptoProvider`]
//! is always implemented by the cryptography crate because it's a trait from Rustls. RustPython
//! needs some extra data such as all of the cipher suites supported by an implementation.
//! The [`CryptoExt`] table stores that extra data if it exists and provides convenience methods
//! as a fallback.
//!
//! Both the [`CryptoProvider`] and [`CryptoExt`] are process-level structs that need to be
//! set before any TLS operations. [`CryptoExt::set_provider`] is thread-safe and runs once.
//! It sets both once per process.

use alloc::sync::Arc;
use std::sync::OnceLock;

use rustls::{
    Error, SignatureScheme, SupportedCipherSuite,
    crypto::{CryptoProvider, SupportedKxGroup},
    pki_types::PrivateKeyDer,
    server::ProducesTickets,
    sign::SigningKey,
};

static CRYPTO_EXT: OnceLock<CryptoExt> = OnceLock::new();

#[derive(Clone, Copy)]
pub struct CryptoExt {
    pub all_cipher_suites: Option<&'static [SupportedCipherSuite]>,
    pub all_kx_groups: Option<&'static [&'static dyn SupportedKxGroup]>,
    #[allow(clippy::type_complexity)]
    pub any_supported_key: Option<fn(&PrivateKeyDer<'_>) -> Result<Arc<dyn SigningKey>, Error>>,
    pub ticketer: fn() -> Result<Arc<dyn ProducesTickets>, Error>,
}

impl CryptoExt {
    #[inline]
    #[must_use]
    pub fn get_ext() -> &'static Self {
        CRYPTO_EXT
            .get()
            .expect("A CryptoProvider must be set before TLS")
    }

    #[inline]
    #[must_use]
    pub fn get_provider() -> &'static CryptoProvider {
        CryptoProvider::get_default().expect("A CryptoProvider must be set before TLS")
    }

    /// Returns all [`SupportedCipherSuite`] or the provider's defaults.
    ///
    /// # Panics
    /// Panics if a [`CryptoProvider`] hasn't been set.
    #[must_use]
    pub fn all_ciphers_or_default(&self) -> &'static [SupportedCipherSuite] {
        self.all_cipher_suites.unwrap_or_else(|| {
            CryptoProvider::get_default()
                .expect("A CryptoProvider has been set if CryptoExt is set")
                .cipher_suites
                .as_slice()
        })
    }

    /// Returns all [`SupportedKxGroup`] or the provider's defaults.
    ///
    /// # Panics
    /// Panics if a [`CryptoProvider`] hasn't been set.
    #[must_use]
    pub fn all_kx_or_default(&self) -> &'static [&'static dyn SupportedKxGroup] {
        self.all_kx_groups.unwrap_or_else(|| {
            CryptoProvider::get_default()
                .expect("A CryptoProvider has been set if CryptoExt is set")
                .kx_groups
                .as_slice()
        })
    }

    /// Return the first supported [`SigningKey`] for a [`PrivateKeyDer`].
    ///
    /// Ideally, this function should be provided by the backend implementation or
    /// the user. This fallback filters out insecure algorithms then picks the first available key
    /// if it exists.
    ///
    /// # Panics
    /// Panics if a [`CryptoProvider`] hasn't been set.
    pub fn any_supported_key(&self, der: &PrivateKeyDer<'_>) -> Result<Arc<dyn SigningKey>, Error> {
        self.any_supported_key.map_or_else(
            || {
                let provider = CryptoProvider::get_default()
                    .expect("A CryptoProvider has been set if CryptoExt is set");
                let key = provider.key_provider.load_private_key(der.clone_key())?;

                for scheme in provider
                    .signature_verification_algorithms
                    .mapping
                    .iter()
                    .filter_map(|(scheme, _)| {
                        (!matches!(
                            scheme,
                            SignatureScheme::RSA_PKCS1_SHA1
                                | SignatureScheme::ECDSA_SHA1_Legacy
                                | SignatureScheme::Unknown(_),
                        ))
                        .then_some(*scheme)
                    })
                {
                    if key.choose_scheme(&[scheme]).is_some() {
                        return Ok(key);
                    }
                }

                Err(Error::General(
                    "failed to parse private key as RSA, ECDSA, or EdDSA".into(),
                ))
            },
            |f| f(der),
        )
    }

    /// Set a process-level [`CryptoProvider`] and [`CryptoExt`].
    ///
    /// A provider must be set before any cryptographic operations. All crypto ops panic if a provider
    /// is unset.
    pub fn set_provider(provider: CryptoProvider, extension: Self) -> Result<(), Error> {
        provider
            .install_default()
            .map_err(|_| Error::General("A default CryptoProvider is already set".into()))?;
        CRYPTO_EXT
            .set(extension)
            .map_err(|_| Error::General("A CryptoExt is already set".into()))
    }
}
