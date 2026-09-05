// spell-checker: ignore pycacert

//! Connection-owned inputs for reconstructing WebPKI's verified path.

use alloc::sync::Arc;
use rustls::{
    RootCertStore,
    crypto::WebPkiSupportedAlgorithms,
    pki_types::{CertificateDer, CertificateRevocationListDer, UnixTime},
};

use super::_ssl::VERIFY_X509_PARTIAL_CHAIN;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Purpose {
    Unverified,
    ServerAuth,
    ClientAuth,
}

/// Like Pyre's `VerifiedChainBuilder`, retain the configuration used by the
/// connection. Reading a mutable SSLContext after the handshake can select a
/// different root, and issuer names alone do not establish a verified path.
#[derive(Debug)]
pub(super) struct VerifiedChainBuilder {
    pub purpose: Purpose,
    pub roots: RootCertStore,
    pub root_der: Vec<Vec<u8>>,
    pub crls: Vec<CertificateRevocationListDer<'static>>,
    pub only_end_entity_revocation: bool,
    pub supported: WebPkiSupportedAlgorithms,
    pub allow_trusted_leaf: bool,
    pub verify_flags: i32,
}

pub(super) type ServerConfig = (Arc<rustls::ServerConfig>, Arc<VerifiedChainBuilder>);
pub(super) type ClientConfig = (Arc<rustls::ClientConfig>, Arc<VerifiedChainBuilder>);

impl VerifiedChainBuilder {
    pub(super) fn build(
        &self,
        peer_chain: &[CertificateDer<'_>],
        now: UnixTime,
    ) -> Option<Vec<Vec<u8>>> {
        self.build_path(peer_chain, now).or_else(|| {
            // OpenSSL still attempts to build a chain under CERT_NONE; a
            // verification failure does not prevent returning the peer chain.
            (self.purpose == Purpose::Unverified && !peer_chain.is_empty()).then(|| {
                peer_chain
                    .iter()
                    .map(|cert| cert.as_ref().to_vec())
                    .collect()
            })
        })
    }

    fn build_path(&self, peer_chain: &[CertificateDer<'_>], now: UnixTime) -> Option<Vec<Vec<u8>>> {
        let (end_entity, intermediates) = peer_chain.split_first()?;

        let certificate = webpki::EndEntityCert::try_from(end_entity).ok()?;
        let parsed_crls = self
            .crls
            .iter()
            .map(|crl| {
                webpki::BorrowedCertRevocationList::from_der(crl.as_ref())
                    .map(webpki::CertRevocationList::from)
            })
            .collect::<Result<Vec<_>, _>>()
            .ok()?;
        let crl_refs: Vec<_> = parsed_crls.iter().collect();
        let revocation = if crl_refs.is_empty() {
            None
        } else {
            let builder = webpki::RevocationOptionsBuilder::new(&crl_refs)
                .ok()?
                .with_expiration_policy(webpki::ExpirationPolicy::Ignore);
            let builder = if self.only_end_entity_revocation {
                builder.with_depth(webpki::RevocationCheckDepth::EndEntity)
            } else {
                builder
            };
            Some(builder.build())
        };
        let usage = match self.purpose {
            Purpose::ServerAuth | Purpose::Unverified => webpki::KeyUsage::server_auth(),
            Purpose::ClientAuth => webpki::KeyUsage::client_auth(),
        };
        let path = certificate.verify_for_usage(
            self.supported.all,
            &self.roots.roots,
            intermediates,
            now,
            usage,
            revocation,
            None,
        );
        let Ok(path) = path else {
            // PartialChainVerifier also accepts an explicitly trusted leaf.
            // This is only used after the connection's verifier has succeeded.
            if self.allow_trusted_leaf
                && self
                    .root_der
                    .iter()
                    .any(|cert| cert.as_slice() == end_entity.as_ref())
                && x509_parser::parse_x509_certificate(end_entity.as_ref()).is_ok_and(
                    |(_, cert)| {
                        cert.subject() == cert.issuer()
                            || self.verify_flags & VERIFY_X509_PARTIAL_CHAIN != 0
                    },
                )
            {
                return Some(vec![end_entity.as_ref().to_vec()]);
            }
            return None;
        };

        // CertLoader retains DER even when RootCertStore rejects an entry, so
        // the two vectors need not have matching indices. Match the complete
        // selected trust anchor, including its public key and name constraints.
        let anchor_der = self.root_der.iter().find(|der| {
            webpki::anchor_from_trusted_cert(&CertificateDer::from(der.as_slice()))
                .is_ok_and(|anchor| &anchor == path.anchor())
        });
        let mut chain = vec![end_entity.as_ref().to_vec()];
        chain.extend(
            path.intermediate_certificates()
                .map(|cert| cert.der().as_ref().to_vec()),
        );
        // The Mozilla fallback supplies only TrustAnchors, not full DER.
        // Preserve the verified leaf/intermediates when no root DER exists.
        if let Some(anchor_der) = anchor_der
            && chain.last() != Some(anchor_der)
        {
            chain.push(anchor_der.clone());
        }
        Some(chain)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::time::Duration;

    fn certificate(mut pem: &[u8]) -> CertificateDer<'static> {
        rustls_pemfile::certs(&mut pem).next().unwrap().unwrap()
    }

    fn leaf() -> CertificateDer<'static> {
        certificate(include_bytes!("../../../../Lib/test/certdata/keycert3.pem"))
    }

    fn root() -> CertificateDer<'static> {
        certificate(include_bytes!("../../../../Lib/test/certdata/pycacert.pem"))
    }

    fn builder(root_der: Vec<Vec<u8>>) -> VerifiedChainBuilder {
        let mut roots = RootCertStore::empty();
        for der in &root_der {
            let _ = roots.add(CertificateDer::from(der.clone()));
        }
        VerifiedChainBuilder {
            purpose: Purpose::ServerAuth,
            roots,
            root_der,
            crls: Vec::new(),
            only_end_entity_revocation: false,
            supported: rustls::crypto::aws_lc_rs::default_provider()
                .signature_verification_algorithms,
            allow_trusted_leaf: false,
            verify_flags: 0,
        }
    }

    fn verification_time() -> UnixTime {
        // The vendored test certificates are valid at this fixed instant.
        UnixTime::since_unix_epoch(Duration::from_secs(1_700_000_000))
    }

    #[test]
    fn selected_anchor_matches_key_not_just_issuer_name() {
        let root = root();
        let mut wrong_root = root.as_ref().to_vec();
        let (_, parsed) = x509_parser::parse_x509_certificate(&root).unwrap();
        let key = parsed.public_key().raw;
        let key_offset = root
            .windows(key.len())
            .position(|bytes| bytes == key)
            .unwrap();
        // Change a modulus byte while preserving the certificate's subject and
        // DER structure. Trust anchors are inputs; their self-signature is not
        // verified, but this key cannot verify the leaf's signature.
        wrong_root[key_offset + key.len() / 2] ^= 1;
        let builder = builder(vec![wrong_root, root.as_ref().to_vec()]);
        assert_eq!(builder.roots.len(), 2);
        assert_eq!(
            builder.build(&[leaf()], verification_time()).unwrap(),
            vec![leaf().as_ref().to_vec(), root.as_ref().to_vec()],
        );
    }

    #[test]
    fn unused_peer_certificates_are_not_part_of_the_verified_path() {
        let root = root();
        let unrelated = certificate(include_bytes!("../../../../Lib/test/certdata/keycert.pem"));
        let builder = builder(vec![root.as_ref().to_vec()]);
        assert_eq!(
            builder
                .build(&[leaf(), unrelated], verification_time())
                .unwrap(),
            vec![leaf().as_ref().to_vec(), root.as_ref().to_vec()],
        );
    }

    #[test]
    fn rejected_root_der_does_not_shift_the_selected_anchor() {
        let root = root();
        let builder = builder(vec![vec![0], root.as_ref().to_vec()]);
        assert_eq!(builder.roots.len(), 1);
        assert_eq!(
            builder
                .build(&[leaf()], verification_time())
                .unwrap()
                .last(),
            Some(&root.as_ref().to_vec())
        );
    }

    #[test]
    fn unverified_connections_without_an_issuer_return_the_presented_chain() {
        let mut builder = builder(Vec::new());
        builder.purpose = Purpose::Unverified;
        assert_eq!(
            builder.build(&[leaf()], verification_time()),
            Some(vec![leaf().as_ref().to_vec()])
        );
    }

    #[test]
    fn unverified_connections_can_build_from_configured_roots() {
        let root = root();
        let mut builder = builder(vec![root.as_ref().to_vec()]);
        builder.purpose = Purpose::Unverified;
        assert_eq!(
            builder
                .build(&[leaf()], verification_time())
                .unwrap()
                .last(),
            Some(&root.as_ref().to_vec())
        );
    }

    #[test]
    fn anchor_without_der_preserves_the_verified_leaf() {
        let mut builder = builder(vec![root().as_ref().to_vec()]);
        builder.root_der.clear();
        assert_eq!(
            builder.build(&[leaf()], verification_time()),
            Some(vec![leaf().as_ref().to_vec()])
        );
    }

    #[test]
    fn client_auth_uses_the_client_certificate_purpose() {
        let mut builder = builder(vec![root().as_ref().to_vec()]);
        builder.purpose = Purpose::ClientAuth;
        assert_eq!(
            builder.build(&[leaf()], verification_time()).unwrap().len(),
            2
        );
    }

    #[test]
    fn explicitly_trusted_leaf_has_a_one_certificate_path() {
        let leaf = leaf();
        let mut builder = builder(vec![leaf.as_ref().to_vec()]);
        builder.allow_trusted_leaf = true;
        builder.verify_flags = VERIFY_X509_PARTIAL_CHAIN;
        assert_eq!(
            builder.build(core::slice::from_ref(&leaf), verification_time()),
            Some(vec![leaf.as_ref().to_vec()])
        );
    }
}
