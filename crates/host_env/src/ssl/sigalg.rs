// cspell: ignore sigalgs rsae mldsa

//! Per-context TLS server signature algorithm selection.

use alloc::sync::Arc;
use rustls::{
    DigitallySignedStruct, DistinguishedName, Error, SignatureAlgorithm, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, ServerName, SubjectPublicKeyInfoDer, UnixTime},
    server::{ClientHello, ResolvesServerCert},
    sign::{CertifiedKey, Signer, SigningKey},
};

use super::providers::CryptoExt;

pub fn scheme_name(scheme: SignatureScheme) -> Option<&'static str> {
    use SignatureScheme::{
        ECDSA_NISTP256_SHA256, ECDSA_NISTP384_SHA384, ECDSA_NISTP521_SHA512, ECDSA_SHA1_Legacy,
        ED448, ED25519, ML_DSA_44, ML_DSA_65, ML_DSA_87, RSA_PKCS1_SHA1, RSA_PKCS1_SHA256,
        RSA_PKCS1_SHA384, RSA_PKCS1_SHA512, RSA_PSS_SHA256, RSA_PSS_SHA384, RSA_PSS_SHA512,
    };
    Some(match scheme {
        RSA_PKCS1_SHA1 => "rsa_pkcs1_sha1",
        ECDSA_SHA1_Legacy => "ecdsa_sha1",
        RSA_PKCS1_SHA256 => "rsa_pkcs1_sha256",
        RSA_PKCS1_SHA384 => "rsa_pkcs1_sha384",
        RSA_PKCS1_SHA512 => "rsa_pkcs1_sha512",
        ECDSA_NISTP256_SHA256 => "ecdsa_secp256r1_sha256",
        ECDSA_NISTP384_SHA384 => "ecdsa_secp384r1_sha384",
        ECDSA_NISTP521_SHA512 => "ecdsa_secp521r1_sha512",
        RSA_PSS_SHA256 => "rsa_pss_rsae_sha256",
        RSA_PSS_SHA384 => "rsa_pss_rsae_sha384",
        RSA_PSS_SHA512 => "rsa_pss_rsae_sha512",
        ED25519 => "ed25519",
        ED448 => "ed448",
        ML_DSA_44 => "mldsa44",
        ML_DSA_65 => "mldsa65",
        ML_DSA_87 => "mldsa87",
        _ => return None,
    })
}

pub fn parse_list(list: &str) -> Result<Vec<SignatureScheme>, &'static str> {
    let supported = CryptoExt::get_provider()
        .signature_verification_algorithms
        .supported_schemes();
    let mut selected = Vec::new();
    for name in list.split(':') {
        let (name, optional) = name.strip_prefix('?').map_or((name, false), |n| (n, true));
        let name = match name {
            "RSA+SHA256" => "rsa_pkcs1_sha256",
            "RSA+SHA384" => "rsa_pkcs1_sha384",
            "RSA+SHA512" => "rsa_pkcs1_sha512",
            "ECDSA+SHA256" => "ecdsa_secp256r1_sha256",
            "ECDSA+SHA384" => "ecdsa_secp384r1_sha384",
            "ECDSA+SHA512" => "ecdsa_secp521r1_sha512",
            other => other,
        };
        match supported.iter().find(|s| scheme_name(**s) == Some(name)) {
            Some(scheme) if !selected.contains(scheme) => selected.push(*scheme),
            Some(_) => {}
            None if optional => {}
            None => return Err("unrecognized signature algorithm"),
        }
    }
    if selected.is_empty() {
        return Err("unrecognized signature algorithm");
    }
    Ok(selected)
}

/// Restrict advertisement and handshake signatures, retaining the complete
/// certificate verifier (including trust, hostname and revocation checks).
#[derive(Debug)]
pub struct ServerVerifier {
    pub inner: Arc<dyn ServerCertVerifier>,
    pub schemes: Vec<SignatureScheme>,
}

impl ServerVerifier {
    fn check_scheme(&self, dss: &DigitallySignedStruct) -> Result<(), Error> {
        if self.schemes.contains(&dss.scheme) {
            Ok(())
        } else {
            Err(Error::General(
                "server signature algorithm is not enabled".into(),
            ))
        }
    }
}

impl ServerCertVerifier for ServerVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        self.inner
            .verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        self.check_scheme(dss)?;
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        self.check_scheme(dss)?;
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        let supported = self.inner.supported_verify_schemes();
        self.schemes
            .iter()
            .filter(|s| supported.contains(s))
            .copied()
            .collect()
    }

    fn requires_raw_public_keys(&self) -> bool {
        self.inner.requires_raw_public_keys()
    }

    fn root_hint_subjects(&self) -> Option<&[DistinguishedName]> {
        self.inner.root_hint_subjects()
    }
}

#[derive(Debug)]
struct RestrictedSigningKey {
    inner: Arc<dyn SigningKey>,
    schemes: Vec<SignatureScheme>,
}

impl SigningKey for RestrictedSigningKey {
    fn choose_scheme(&self, offered: &[SignatureScheme]) -> Option<Box<dyn Signer>> {
        self.schemes
            .iter()
            .filter(|s| offered.contains(s))
            .find_map(|s| self.inner.choose_scheme(&[*s]))
    }

    fn public_key(&self) -> Option<SubjectPublicKeyInfoDer<'_>> {
        self.inner.public_key()
    }

    fn algorithm(&self) -> SignatureAlgorithm {
        self.inner.algorithm()
    }
}

#[derive(Debug)]
pub struct ServerResolver {
    pub inner: Arc<dyn ResolvesServerCert>,
    pub schemes: Vec<SignatureScheme>,
}

pub fn restrict_key(key: &CertifiedKey, schemes: &[SignatureScheme]) -> Arc<CertifiedKey> {
    let mut key = key.clone();
    key.key = Arc::new(RestrictedSigningKey {
        inner: key.key,
        schemes: schemes.to_vec(),
    });
    Arc::new(key)
}

impl ResolvesServerCert for ServerResolver {
    fn resolve(&self, hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        let key = self.inner.resolve(hello)?;
        Some(restrict_key(&key, &self.schemes))
    }

    fn only_raw_public_keys(&self) -> bool {
        self.inner.only_raw_public_keys()
    }
}
