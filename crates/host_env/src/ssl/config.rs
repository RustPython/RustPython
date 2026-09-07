//! rustls ClientConfig / ServerConfig construction for `_ssl`.

use alloc::sync::Arc;
use parking_lot::RwLock as ParkingRwLock;
use rustls::client::ClientConfig;
use rustls::crypto::{CryptoProvider, SupportedKxGroup};
use rustls::pki_types::{CertificateDer, CertificateRevocationListDer, PrivateKeyDer};
use rustls::server::{ProducesTickets, ResolvesServerCert, ServerConfig, WebPkiClientVerifier};
use rustls::sign::CertifiedKey;
use rustls::{RootCertStore, SupportedCipherSuite};

use super::chain::{self, Purpose, VerifiedChainBuilder};
use super::cipher;
use super::constants::{VERIFY_X509_PARTIAL_CHAIN, VERIFY_X509_STRICT};
use super::providers::CryptoExt;

const X509_V_FLAG_CRL_CHECK: i32 = 4;

/// Common protocol settings shared between client and server connections
#[derive(Debug)]
pub struct ProtocolSettings {
    pub versions: &'static [&'static rustls::SupportedProtocolVersion],
    pub kx_groups: Option<Vec<&'static dyn rustls::crypto::SupportedKxGroup>>,
    pub cipher_suites: Option<Vec<rustls::SupportedCipherSuite>>,
    pub alpn_protocols: Vec<Vec<u8>>,
}

/// Options for creating a server TLS configuration
#[derive(Debug)]
pub struct ServerConfigOptions {
    /// Common protocol settings (versions, ALPN, KX groups, cipher suites)
    pub protocol_settings: ProtocolSettings,
    /// Server certificate chain
    pub cert_chain: Vec<CertificateDer<'static>>,
    /// Server private key
    pub private_key: PrivateKeyDer<'static>,
    /// Root certificates for client verification (if required)
    pub root_store: Option<RootCertStore>,
    pub ca_certs_der: Vec<Vec<u8>>,
    /// Whether to request client certificate
    pub request_client_cert: bool,
    /// Whether to use deferred client certificate validation (TLS 1.3)
    pub use_deferred_validation: bool,
    /// Custom certificate resolver (for SNI support)
    pub cert_resolver: Option<Arc<dyn ResolvesServerCert>>,
    /// Deferred certificate error storage (for TLS 1.3)
    pub deferred_cert_error: Option<Arc<ParkingRwLock<Option<String>>>>,
    /// Session storage for server-side session resumption
    pub session_storage: Option<Arc<rustls::server::ServerSessionMemoryCache>>,
    /// Shared ticketer for TLS 1.2 session tickets (stateless resumption)
    pub ticketer: Option<Arc<dyn ProducesTickets>>,
}

/// Options for creating a client TLS configuration
#[derive(Debug)]
pub struct ClientConfigOptions {
    /// Common protocol settings (versions, ALPN, KX groups, cipher suites)
    pub protocol_settings: ProtocolSettings,
    /// Root certificates for server verification
    pub root_store: Option<RootCertStore>,
    /// DER-encoded CA certificates (for partial chain verification)
    pub ca_certs_der: Vec<Vec<u8>>,
    /// Client certificate chain (for mTLS)
    pub cert_chain: Option<Vec<CertificateDer<'static>>>,
    /// Client private key (for mTLS)
    pub private_key: Option<PrivateKeyDer<'static>>,
    /// Whether to verify server certificates (CERT_NONE disables verification)
    pub verify_server_cert: bool,
    /// Whether to check hostname against certificate (check_hostname)
    pub check_hostname: bool,
    /// SSL verification flags (e.g., VERIFY_X509_STRICT)
    pub verify_flags: i32,
    /// Session store for client-side session resumption
    pub session_store: Option<Arc<dyn rustls::client::ClientSessionStore>>,
    /// Certificate Revocation Lists for CRL checking
    pub crls: Vec<CertificateRevocationListDer<'static>>,
}

/// Create custom CryptoProvider with specified cipher suites and key exchange groups
///
/// This helper function consolidates the duplicated CryptoProvider creation logic
/// for both server and client configurations.
fn create_custom_crypto_provider(
    cipher_suites: Option<Vec<SupportedCipherSuite>>,
    kx_groups: Option<Vec<&'static dyn SupportedKxGroup>>,
) -> Arc<CryptoProvider> {
    let default_provider = CryptoExt::get_provider();

    Arc::new(CryptoProvider {
        cipher_suites: cipher_suites.unwrap_or_else(|| default_provider.cipher_suites.clone()),
        kx_groups: kx_groups.unwrap_or_else(|| default_provider.kx_groups.clone()),
        signature_verification_algorithms: default_provider.signature_verification_algorithms,
        secure_random: default_provider.secure_random,
        key_provider: default_provider.key_provider,
    })
}

/// Create a server TLS configuration
///
/// This abstracts the complex rustls ServerConfig building logic,
/// matching SSL_CTX initialization for server sockets.
pub fn create_server_config(options: ServerConfigOptions) -> Result<chain::ServerConfig, String> {
    // Create custom crypto provider using helper function
    let custom_provider = create_custom_crypto_provider(
        options.protocol_settings.cipher_suites.clone(),
        options.protocol_settings.kx_groups.clone(),
    );

    let chain_builder = Arc::new(VerifiedChainBuilder {
        purpose: if options.request_client_cert {
            Purpose::ClientAuth
        } else {
            Purpose::Unverified
        },
        roots: options
            .root_store
            .clone()
            .unwrap_or_else(RootCertStore::empty),
        root_der: options.ca_certs_der,
        crls: Vec::new(),
        only_end_entity_revocation: false,
        supported: custom_provider.signature_verification_algorithms,
        allow_trusted_leaf: false,
        verify_flags: 0,
    });

    // Step 1: Build the appropriate client cert verifier based on settings
    let client_cert_verifier: Option<Arc<dyn rustls::server::danger::ClientCertVerifier>> =
        if let Some(root_store) = options.root_store {
            if options.request_client_cert {
                // Client certificate verification required
                let base_verifier = WebPkiClientVerifier::builder_with_provider(
                    Arc::new(root_store),
                    custom_provider.clone(),
                )
                .build()
                .map_err(|e| format!("Failed to create client verifier: {e}"))?;

                if options.use_deferred_validation {
                    // TLS 1.3: Use deferred validation
                    if let Some(deferred_error) = options.deferred_cert_error {
                        use super::verify::DeferredClientCertVerifier;
                        let deferred_verifier =
                            DeferredClientCertVerifier::new(base_verifier, deferred_error);
                        Some(Arc::new(deferred_verifier))
                    } else {
                        // No deferred error storage provided, use immediate validation
                        Some(base_verifier)
                    }
                } else {
                    // TLS 1.2 or non-deferred: Use immediate validation
                    Some(base_verifier)
                }
            } else {
                // No client authentication
                None
            }
        } else {
            // No root store - no client authentication
            None
        };

    // Step 2: Create ServerConfig builder once with the selected verifier
    let builder = ServerConfig::builder_with_provider(custom_provider)
        .with_protocol_versions(options.protocol_settings.versions)
        .map_err(|e| format!("Failed to create server config builder: {e}"))?;

    let builder = if let Some(verifier) = client_cert_verifier {
        builder.with_client_cert_verifier(verifier)
    } else {
        builder.with_no_client_auth()
    };

    // Add certificate
    let mut config = if let Some(resolver) = options.cert_resolver {
        // Use custom cert resolver (e.g., for SNI)
        builder.with_cert_resolver(resolver)
    } else {
        // Use single certificate
        builder
            .with_single_cert(options.cert_chain, options.private_key)
            .map_err(|e| format!("Failed to set server certificate: {e}"))?
    };

    // Set ALPN protocols with fallback
    apply_alpn_with_fallback(
        &mut config.alpn_protocols,
        &options.protocol_settings.alpn_protocols,
    );

    // Set session storage for server-side session resumption (TLS 1.3)
    if let Some(session_storage) = options.session_storage {
        config.session_storage = session_storage;
    }

    // Set ticketer for TLS 1.2 session tickets (stateless resumption)
    if let Some(ticketer) = options.ticketer {
        config.ticketer = ticketer.clone();
    }

    Ok((Arc::new(config), chain_builder))
}

/// Build WebPki verifier with CRL support
///
/// This helper function consolidates the duplicated CRL setup logic for both
/// check_hostname=True and check_hostname=False cases.
fn build_webpki_verifier_with_crls(
    root_store: Arc<RootCertStore>,
    crls: Vec<CertificateRevocationListDer<'static>>,
    verify_flags: i32,
) -> Result<Arc<dyn rustls::client::danger::ServerCertVerifier>, String> {
    use rustls::client::WebPkiServerVerifier;

    let mut verifier_builder = WebPkiServerVerifier::builder(root_store);

    // Check if CRL verification is requested
    let crl_check_requested = verify_flags & X509_V_FLAG_CRL_CHECK != 0;
    let has_crls = !crls.is_empty();

    // Add CRLs if provided OR if CRL checking is explicitly requested
    // (even with empty CRLs, rustls will fail verification if CRL checking is enabled)
    if has_crls || crl_check_requested {
        verifier_builder = verifier_builder.with_crls(crls);

        // Check if we should only verify end-entity (leaf) certificates
        if verify_flags & X509_V_FLAG_CRL_CHECK != 0 {
            verifier_builder = verifier_builder.only_check_end_entity_revocation();
        }
    }

    let webpki_verifier = verifier_builder
        .build()
        .map_err(|e| format!("Failed to build WebPkiServerVerifier: {e}"))?;

    Ok(webpki_verifier as Arc<dyn rustls::client::danger::ServerCertVerifier>)
}

/// Apply verifier wrappers (CRLCheckVerifier and StrictCertVerifier)
///
/// This helper function consolidates the duplicated verifier wrapping logic.
fn apply_verifier_wrappers(
    verifier: Arc<dyn rustls::client::danger::ServerCertVerifier>,
    verify_flags: i32,
    has_crls: bool,
    ca_certs_der: Vec<Vec<u8>>,
) -> Arc<dyn rustls::client::danger::ServerCertVerifier> {
    let crl_check_requested = verify_flags & X509_V_FLAG_CRL_CHECK != 0;

    // Wrap with CRLCheckVerifier to enforce CRL checking when flags are set
    let verifier = if crl_check_requested {
        use super::verify::CRLCheckVerifier;
        Arc::new(CRLCheckVerifier::new(
            verifier,
            has_crls,
            crl_check_requested,
        ))
    } else {
        verifier
    };

    // Always use PartialChainVerifier when trust store is not empty
    // This allows self-signed certificates in trust store to be trusted
    // (OpenSSL behavior: self-signed certs are always trusted, non-self-signed require flag)
    let verifier = if !ca_certs_der.is_empty() {
        use super::verify::PartialChainVerifier;
        Arc::new(PartialChainVerifier::new(
            verifier,
            ca_certs_der,
            verify_flags,
        ))
    } else {
        verifier
    };

    // Wrap with StrictCertVerifier if VERIFY_X509_STRICT flag is set
    if verify_flags & VERIFY_X509_STRICT != 0 {
        Arc::new(super::verify::StrictCertVerifier::new(
            verifier,
            verify_flags,
        ))
    } else {
        verifier
    }
}

/// Apply ALPN protocols
///
/// OpenSSL 1.1.0f+ allows ALPN negotiation to fail without aborting handshake.
/// rustls follows RFC 7301 strictly and rejects connections with no matching protocol.
/// To emulate OpenSSL behavior, we add a special fallback protocol (null byte).
fn apply_alpn_with_fallback(config_alpn: &mut Vec<Vec<u8>>, alpn_protocols: &[Vec<u8>]) {
    if !alpn_protocols.is_empty() {
        *config_alpn = alpn_protocols.to_vec();
        config_alpn.push(vec![0u8]); // Add null byte as fallback marker
    }
}

/// Create a client TLS configuration
///
/// This abstracts the complex rustls ClientConfig building logic,
/// matching SSL_CTX initialization for client sockets.
pub fn create_client_config(options: ClientConfigOptions) -> Result<chain::ClientConfig, String> {
    // Create custom crypto provider using helper function
    let custom_provider = create_custom_crypto_provider(
        options.protocol_settings.cipher_suites.clone(),
        options.protocol_settings.kx_groups.clone(),
    );

    let chain_builder = Arc::new(VerifiedChainBuilder {
        purpose: if options.verify_server_cert {
            Purpose::ServerAuth
        } else {
            Purpose::Unverified
        },
        roots: options
            .root_store
            .clone()
            .unwrap_or_else(RootCertStore::empty),
        root_der: options.ca_certs_der.clone(),
        crls: options.crls.clone(),
        only_end_entity_revocation: options.verify_flags & X509_V_FLAG_CRL_CHECK != 0,
        supported: custom_provider.signature_verification_algorithms,
        allow_trusted_leaf: options.check_hostname
            || options.verify_flags & VERIFY_X509_PARTIAL_CHAIN != 0,
        verify_flags: options.verify_flags,
    });

    // Step 1: Build the appropriate verifier based on verification settings
    let verifier: Arc<dyn rustls::client::danger::ServerCertVerifier> = if options
        .verify_server_cert
    {
        // Verify server certificates
        let root_store = options
            .root_store
            .ok_or("Root store required for server verification")?;

        let root_store_arc = Arc::new(root_store);

        // Check if root_store is empty (no CA certs loaded)
        // CPython allows this and fails during handshake with SSLCertVerificationError
        if root_store_arc.is_empty() {
            // Use EmptyRootStoreVerifier - always fails with UnknownIssuer during handshake
            use super::verify::EmptyRootStoreVerifier;
            Arc::new(EmptyRootStoreVerifier)
        } else {
            // Calculate has_crls once for both hostname verification paths
            let has_crls = !options.crls.is_empty();

            if options.check_hostname {
                // Default behavior: verify both certificate chain and hostname
                let base_verifier = build_webpki_verifier_with_crls(
                    root_store_arc,
                    options.crls,
                    options.verify_flags,
                )?;

                // Apply CRL and Strict verifier wrappers using helper function
                apply_verifier_wrappers(
                    base_verifier,
                    options.verify_flags,
                    has_crls,
                    options.ca_certs_der.clone(),
                )
            } else {
                // check_hostname=False: verify certificate chain but ignore hostname
                use super::verify::HostnameIgnoringVerifier;

                // Build verifier with CRL support using helper function
                let webpki_verifier = build_webpki_verifier_with_crls(
                    root_store_arc,
                    options.crls,
                    options.verify_flags,
                )?;

                // Apply CRL verifier wrapper if needed (without Strict wrapper yet)
                let crl_check_requested = options.verify_flags & X509_V_FLAG_CRL_CHECK != 0;
                let verifier = if crl_check_requested {
                    use super::verify::CRLCheckVerifier;
                    Arc::new(CRLCheckVerifier::new(
                        webpki_verifier,
                        has_crls,
                        crl_check_requested,
                    )) as Arc<dyn rustls::client::danger::ServerCertVerifier>
                } else {
                    webpki_verifier
                };

                // Wrap with PartialChainVerifier if VERIFY_X509_PARTIAL_CHAIN is set
                let verifier = if options.verify_flags & VERIFY_X509_PARTIAL_CHAIN != 0 {
                    use super::verify::PartialChainVerifier;
                    Arc::new(PartialChainVerifier::new(
                        verifier,
                        options.ca_certs_der.clone(),
                        options.verify_flags,
                    )) as Arc<dyn rustls::client::danger::ServerCertVerifier>
                } else {
                    verifier
                };

                // Wrap with HostnameIgnoringVerifier to bypass hostname checking
                let hostname_ignoring_verifier: Arc<
                    dyn rustls::client::danger::ServerCertVerifier,
                > = Arc::new(HostnameIgnoringVerifier::new_with_verifier(verifier));

                // Apply Strict verifier wrapper once at the end if needed
                if options.verify_flags & VERIFY_X509_STRICT != 0 {
                    Arc::new(super::verify::StrictCertVerifier::new(
                        hostname_ignoring_verifier,
                        options.verify_flags,
                    ))
                } else {
                    hostname_ignoring_verifier
                }
            }
        }
    } else {
        // CERT_NONE: disable all verification
        use super::verify::NoVerifier;
        Arc::new(NoVerifier)
    };

    // Step 2: Create ClientConfig builder once with the selected verifier
    let builder = ClientConfig::builder_with_provider(custom_provider)
        .with_protocol_versions(options.protocol_settings.versions)
        .map_err(|e| format!("Failed to create client config builder: {e}"))?
        .dangerous()
        .with_custom_certificate_verifier(verifier);

    // Add client certificate if provided (mTLS)
    let mut config =
        if let (Some(cert_chain), Some(private_key)) = (options.cert_chain, options.private_key) {
            builder
                .with_client_auth_cert(cert_chain, private_key)
                .map_err(|e| format!("Failed to set client certificate: {e}"))?
        } else {
            builder.with_no_client_auth()
        };

    // Set ALPN protocols
    apply_alpn_with_fallback(
        &mut config.alpn_protocols,
        &options.protocol_settings.alpn_protocols,
    );

    // Set session resumption
    if let Some(session_store) = options.session_store {
        use rustls::client::Resumption;
        config.resumption = Resumption::store(session_store);
    }

    Ok((Arc::new(config), chain_builder))
}

// Multi-Certificate Resolver for RSA/ECC Support

/// Multi-certificate resolver that selects appropriate certificate based on client capabilities
///
/// This resolver implements OpenSSL's behavior of supporting multiple certificate/key pairs
/// (e.g., one RSA and one ECC) and selecting the appropriate one based on the client's
/// supported signature algorithms during the TLS handshake.
///
/// OpenSSL's SSL_CTX_use_certificate_chain_file can be called multiple
/// times to add different certificate types, and OpenSSL automatically selects the best one.
#[derive(Debug)]
pub struct MultiCertResolver {
    cert_keys: Vec<Arc<CertifiedKey>>,
}

impl MultiCertResolver {
    /// Create a new multi-certificate resolver
    pub fn new(cert_keys: Vec<Arc<CertifiedKey>>) -> Self {
        Self { cert_keys }
    }
}

impl ResolvesServerCert for MultiCertResolver {
    fn resolve(&self, client_hello: rustls::server::ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        // Get the signature schemes supported by the client
        let client_schemes = client_hello.signature_schemes();

        // Try to find a certificate that matches the client's signature schemes
        for cert_key in &self.cert_keys {
            // Check if this certificate's signing key is compatible with any of the
            // client's supported signature schemes
            if let Some(_scheme) = cert_key.key.choose_scheme(client_schemes) {
                return Some(cert_key.clone());
            }
        }

        // If no perfect match, return the first certificate as fallback
        // (This matches OpenSSL's behavior of using the first loaded cert if negotiation fails)
        self.cert_keys.first().cloned()
    }
}

// Helper Functions for OpenSSL Compatibility:

/// Convert curve name to rustls key exchange group
///
/// Maps OpenSSL curve names (e.g., "prime256v1", "secp384r1") to rustls KxGroups.
/// Returns an error if the curve is not supported by rustls.
pub fn curve_name_to_kx_group(curve: &str) -> Result<Vec<&'static dyn SupportedKxGroup>, String> {
    cipher::kx_group_by_openssl_name(curve)
        .map(|group| vec![group])
        .ok_or_else(|| format!("unknown curve name '{curve}'"))
}
