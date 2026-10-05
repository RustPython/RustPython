//! Server acceptor state used before rustls has a `ServerConfig`.

use core::fmt;
use rustls::server::Acceptor;

/// Server configuration is selected only after receiving ClientHello and
/// invoking SNI. A failed connection stays terminal while its alert drains.
/// ShuttingDown means our close_notify is queued and must not be sent twice.
pub enum TlsState<E> {
    WaitingForClientHello(Box<Acceptor>),
    InProgress,
    Handshaking,
    Connected,
    ShuttingDown,
    ShutDown,
    SendingAlert { error: E },
}

impl<E> TlsState<E> {
    pub fn new(server_side: bool) -> Self {
        if server_side {
            Self::WaitingForClientHello(Box::default())
        } else {
            Self::Handshaking
        }
    }
}

impl<E> fmt::Debug for TlsState<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::WaitingForClientHello(_) => "WaitingForClientHello",
            Self::InProgress => "InProgress",
            Self::Handshaking => "Handshaking",
            Self::Connected => "Connected",
            Self::ShuttingDown => "ShuttingDown",
            Self::ShutDown => "ShutDown",
            Self::SendingAlert { .. } => "SendingAlert",
        })
    }
}

/// SNI rejection happens before ServerHello, so the fatal alert is plaintext
/// even when the ClientHello offers TLS 1.3 (RFC 8446 section 5.1).
#[must_use]
pub fn sni_alert(description: u8) -> Vec<u8> {
    vec![21, 3, 3, 0, 2, 2, description]
}

/// Serialize a failed initial `Accepted::into_connection` attempt.
///
/// No server bytes have been sent at this boundary. Rustls may have queued
/// ServerHello before finding that a full handshake has no usable signature,
/// or may return a signing-key error without queuing any alert (TLS 1.2).
/// Send only a plaintext fatal alert, so the peer cannot start its next flight
/// while the server is closing. Keep certificate/session selection in rustls:
/// resumed sessions do not need a new certificate signature.
pub fn initial_handshake_alert(
    error: &rustls::Error,
    mut alert: rustls::server::AcceptedAlert,
) -> std::io::Result<Vec<u8>> {
    // TLS 1.2's emit_server_kx reports the exact General message below when
    // SigningKey::choose_scheme fails; TLS 1.3 uses PeerIncompatible.
    let signature_mismatch = matches!(
        error,
        rustls::Error::PeerIncompatible(rustls::PeerIncompatible::NoSignatureSchemesInCommon)
    ) || matches!(error, rustls::Error::General(message) if message == "incompatible signing key");
    if signature_mismatch {
        return Ok(sni_alert(u8::from(
            rustls::AlertDescription::HandshakeFailure,
        )));
    }
    let mut bytes = Vec::new();
    alert.write_all(&mut bytes)?;
    Ok(bytes)
}

pub fn feed_acceptor(acceptor: &mut Acceptor, bytes: &[u8]) -> std::io::Result<()> {
    let mut reader = std::io::Cursor::new(bytes);
    while reader.position() < bytes.len() as u64 {
        if acceptor.read_tls(&mut reader)? == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::sync::Arc;

    fn large_client_hello() -> Vec<u8> {
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let mut config = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(rustls::RootCertStore::empty())
            .with_no_client_auth();
        config.alpn_protocols = (0..70).map(|i| format!("{i:0100}").into_bytes()).collect();
        let mut client =
            rustls::ClientConnection::new(Arc::new(config), "localhost".try_into().unwrap())
                .unwrap();
        let mut bytes = Vec::new();
        client.write_tls(&mut bytes).unwrap();
        assert!(bytes.len() > 4096);
        bytes
    }

    #[test]
    fn consumes_large_client_hello_completely() {
        let mut acceptor = Acceptor::default();
        feed_acceptor(&mut acceptor, &large_client_hello()).unwrap();
        let accepted = acceptor.accept().unwrap().unwrap();
        assert_eq!(accepted.client_hello().server_name(), Some("localhost"));
        assert_eq!(accepted.client_hello().alpn().unwrap().count(), 70);
    }

    #[test]
    fn waits_for_every_fragment_of_client_hello() {
        let hello = large_client_hello();
        let mut acceptor = Acceptor::default();
        for byte in &hello[..hello.len() - 1] {
            feed_acceptor(&mut acceptor, &[*byte]).unwrap();
            assert!(acceptor.accept().unwrap().is_none());
        }
        feed_acceptor(&mut acceptor, &hello[hello.len() - 1..]).unwrap();
        assert!(acceptor.accept().unwrap().is_some());
    }

    #[test]
    fn fatal_sni_alert_is_one_plaintext_record() {
        assert_eq!(sni_alert(49), [21, 3, 3, 0, 2, 2, 49]);
        assert_eq!(sni_alert(40), [21, 3, 3, 0, 2, 2, 40]);
        assert_eq!(sni_alert(80), [21, 3, 3, 0, 2, 2, 80]);
    }
}

#[cfg(test)]
mod signature_tests {
    use super::super::config::MultiCertResolver;
    use super::*;
    use alloc::sync::Arc;
    use rustls::{
        ClientConnection, SignatureScheme, client::WebPkiServerVerifier, server::Acceptor,
    };
    use rustls::{RootCertStore, SupportedCipherSuite, crypto::CryptoProvider, sign::CertifiedKey};
    use rustls::{client::ClientConfig, server::ServerConfig};

    fn certificate_key(mut pem: &[u8], provider: &CryptoProvider) -> Arc<CertifiedKey> {
        let mut private_key_pem = pem;
        let certificates = rustls_pemfile::certs(&mut pem)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let key = rustls_pemfile::private_key(&mut private_key_pem)
            .unwrap()
            .unwrap();
        Arc::new(CertifiedKey::from_der(certificates, key, provider).unwrap())
    }

    fn client_config(
        version: &'static rustls::SupportedProtocolVersion,
        scheme: SignatureScheme,
        cipher_suite: Option<SupportedCipherSuite>,
    ) -> Arc<ClientConfig> {
        let mut provider = rustls::crypto::aws_lc_rs::default_provider();
        if let Some(suite) = cipher_suite {
            provider.cipher_suites = vec![suite];
        }
        let provider = Arc::new(provider);
        let mut roots = RootCertStore::empty();
        let ca = include_bytes!("../../../../Lib/test/certdata/pycacert.pem");
        for cert in rustls_pemfile::certs(&mut &ca[..]) {
            roots.add(cert.unwrap()).unwrap();
        }
        let verifier =
            WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider.clone())
                .build()
                .unwrap();
        let config = ClientConfig::builder_with_provider(provider)
            .with_protocol_versions(&[version])
            .unwrap()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(super::super::sigalg::ServerVerifier {
                inner: verifier,
                schemes: vec![scheme],
            }))
            .with_no_client_auth();
        Arc::new(config)
    }

    fn client_hello(config: Arc<ClientConfig>) -> (ClientConnection, rustls::server::Accepted) {
        let mut client = ClientConnection::new(config, "localhost".try_into().unwrap()).unwrap();
        let mut hello = Vec::new();
        client.write_tls(&mut hello).unwrap();
        let mut acceptor = Acceptor::default();
        super::super::handshake::feed_acceptor(&mut acceptor, &hello).unwrap();
        (client, acceptor.accept().unwrap().unwrap())
    }

    fn server_config(
        version: &'static rustls::SupportedProtocolVersion,
        include_incompatible_first_key: bool,
        cipher_suite: Option<SupportedCipherSuite>,
        scheme: SignatureScheme,
    ) -> Arc<ServerConfig> {
        let mut provider = rustls::crypto::aws_lc_rs::default_provider();
        if let Some(suite) = cipher_suite {
            provider.cipher_suites = vec![suite];
        }
        let provider = Arc::new(provider);
        let mut keys = Vec::new();
        if include_incompatible_first_key {
            keys.push(certificate_key(
                include_bytes!("../../../../Lib/test/certdata/keycertecc.pem"),
                &provider,
            ));
        }
        keys.push(certificate_key(
            include_bytes!("../../../../Lib/test/certdata/keycert3.pem"),
            &provider,
        ));
        Arc::new(
            ServerConfig::builder_with_provider(provider)
                .with_protocol_versions(&[version])
                .unwrap()
                .with_no_client_auth()
                .with_cert_resolver(Arc::new(MultiCertResolver::new(keys, Some(&[scheme])))),
        )
    }

    fn assert_signature_mismatch_sends_only_a_fatal_alert(
        version: &'static rustls::SupportedProtocolVersion,
    ) {
        let (mut client, accepted) = client_hello(client_config(
            version,
            SignatureScheme::RSA_PSS_SHA256,
            None,
        ));
        let (error, alert) = accepted
            .into_connection(server_config(
                version,
                false,
                None,
                SignatureScheme::RSA_PSS_SHA384,
            ))
            .unwrap_err();
        let bytes = initial_handshake_alert(&error, alert).unwrap();
        assert!(matches!(
            super::super::error::TlsError::from_rustls(error),
            super::super::error::TlsError::Ssl(_)
        ));
        assert_eq!(bytes.len(), 7, "only a plaintext alert may precede failure");
        assert_eq!(bytes[0], 21, "no ServerHello or CCS may precede the alert");
        assert_eq!(&bytes[3..6], &[0, 2, 2]);
        client.read_tls(&mut &bytes[..]).unwrap();
        assert!(matches!(
            client.process_new_packets(),
            Err(rustls::Error::AlertReceived(_))
        ));
        assert!(
            !client.wants_write(),
            "a rejected client must not queue CCS"
        );
    }

    #[test]
    fn tls12_signature_mismatch_sends_only_a_fatal_alert() {
        assert_signature_mismatch_sends_only_a_fatal_alert(&rustls::version::TLS12);
    }

    #[test]
    fn tls13_signature_mismatch_sends_only_a_fatal_alert() {
        assert_signature_mismatch_sends_only_a_fatal_alert(&rustls::version::TLS13);
    }

    #[test]
    fn unrelated_initial_alert_and_similar_error_text_are_preserved() {
        for override_error in [
            None,
            Some(rustls::Error::General(
                "incompatible signing key detail".into(),
            )),
        ] {
            let (mut client, accepted) = client_hello(client_config(
                &rustls::version::TLS12,
                SignatureScheme::RSA_PSS_SHA384,
                None,
            ));
            let (error, alert) = accepted
                .into_connection(server_config(
                    &rustls::version::TLS13,
                    false,
                    None,
                    SignatureScheme::RSA_PSS_SHA384,
                ))
                .unwrap_err();
            let bytes = initial_handshake_alert(&override_error.unwrap_or(error), alert).unwrap();
            assert_eq!(bytes, sni_alert(70));
            client.read_tls(&mut &bytes[..]).unwrap();
            assert!(matches!(
                client.process_new_packets(),
                Err(rustls::Error::AlertReceived(
                    rustls::AlertDescription::ProtocolVersion
                ))
            ));
        }
    }

    #[test]
    fn disjoint_cipher_suites_keep_the_no_shared_cipher_error() {
        use rustls::crypto::aws_lc_rs::cipher_suite::{
            TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256, TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384,
        };
        let version = &rustls::version::TLS12;
        let (_, accepted) = client_hello(client_config(
            version,
            SignatureScheme::RSA_PSS_SHA384,
            Some(TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256),
        ));
        let (error, _) = accepted
            .into_connection(server_config(
                version,
                false,
                Some(TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384),
                SignatureScheme::RSA_PSS_SHA384,
            ))
            .unwrap_err();
        assert!(matches!(
            super::super::error::TlsError::from_rustls(error),
            super::super::error::TlsError::NoCipherSuites
        ));
    }

    #[test]
    fn incompatible_certificate_algorithm_keeps_the_no_shared_cipher_error() {
        use rustls::crypto::aws_lc_rs::cipher_suite::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256;
        let version = &rustls::version::TLS12;
        let (_, accepted) = client_hello(client_config(
            version,
            SignatureScheme::ECDSA_NISTP256_SHA256,
            Some(TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256),
        ));
        let (error, _) = accepted
            .into_connection(server_config(
                version,
                false,
                None,
                SignatureScheme::RSA_PSS_SHA384,
            ))
            .unwrap_err();
        assert!(matches!(
            super::super::error::TlsError::from_rustls(error),
            super::super::error::TlsError::NoCipherSuites
        ));
    }

    fn finish_handshake(client: &mut ClientConnection, server: &mut rustls::ServerConnection) {
        for _ in 0..16 {
            let mut bytes = Vec::new();
            server.write_tls(&mut bytes).unwrap();
            if !bytes.is_empty() {
                client.read_tls(&mut &bytes[..]).unwrap();
                client.process_new_packets().unwrap();
            }
            bytes.clear();
            client.write_tls(&mut bytes).unwrap();
            if !bytes.is_empty() {
                server.read_tls(&mut &bytes[..]).unwrap();
                server.process_new_packets().unwrap();
            }
            if !client.is_handshaking()
                && !server.is_handshaking()
                && !client.wants_write()
                && !server.wants_write()
            {
                return;
            }
        }
        panic!("in-memory handshake did not finish within 16 exchanges");
    }

    #[test]
    fn signature_match_selects_a_later_compatible_key() {
        for version in [&rustls::version::TLS12, &rustls::version::TLS13] {
            let client_config = client_config(version, SignatureScheme::RSA_PSS_SHA384, None);
            let server_config = server_config(version, true, None, SignatureScheme::RSA_PSS_SHA384);
            for expected_kind in [rustls::HandshakeKind::Full, rustls::HandshakeKind::Resumed] {
                let (mut client, accepted) = client_hello(client_config.clone());
                let mut server = accepted.into_connection(server_config.clone()).unwrap();
                finish_handshake(&mut client, &mut server);
                assert_eq!(client.handshake_kind(), Some(expected_kind));
                assert_eq!(server.handshake_kind(), Some(expected_kind));
            }
        }
    }

    fn assert_resumption_survives_signature_policy_change(
        version: &'static rustls::SupportedProtocolVersion,
    ) {
        let client_config = client_config(version, SignatureScheme::RSA_PSS_SHA256, None);
        let initial_config = server_config(version, false, None, SignatureScheme::RSA_PSS_SHA256);
        let (mut client, accepted) = client_hello(client_config.clone());
        let mut server = accepted.into_connection(initial_config.clone()).unwrap();
        finish_handshake(&mut client, &mut server);
        assert_eq!(client.handshake_kind(), Some(rustls::HandshakeKind::Full));

        // SSLContext retains its server session cache/ticketer when a policy
        // change rebuilds the configuration. Change only the signing policy.
        let mut changed_config = (*initial_config).clone();
        changed_config.cert_resolver =
            server_config(version, false, None, SignatureScheme::RSA_PSS_SHA384)
                .cert_resolver
                .clone();
        let changed_config = Arc::new(changed_config);
        let (mut client, accepted) = client_hello(client_config);
        let mut server = accepted.into_connection(changed_config.clone()).unwrap();
        finish_handshake(&mut client, &mut server);
        assert_eq!(
            client.handshake_kind(),
            Some(rustls::HandshakeKind::Resumed)
        );
        assert_eq!(
            server.handshake_kind(),
            Some(rustls::HandshakeKind::Resumed)
        );

        // The same changed policy must still reject a fresh client's PSS256
        // offer; accepting a resumed session must not relax a full handshake.
        let (mut client, accepted) = client_hello(self::client_config(
            version,
            SignatureScheme::RSA_PSS_SHA256,
            None,
        ));
        let (error, alert) = accepted.into_connection(changed_config).unwrap_err();
        let bytes = initial_handshake_alert(&error, alert).unwrap();
        assert_eq!(bytes, sni_alert(40));
        client.read_tls(&mut &bytes[..]).unwrap();
        assert!(matches!(
            client.process_new_packets(),
            Err(rustls::Error::AlertReceived(
                rustls::AlertDescription::HandshakeFailure,
            ))
        ));
    }

    #[test]
    fn tls12_resumption_survives_signature_policy_change() {
        assert_resumption_survives_signature_policy_change(&rustls::version::TLS12);
    }

    #[test]
    fn tls13_resumption_survives_signature_policy_change() {
        assert_resumption_survives_signature_policy_change(&rustls::version::TLS13);
    }
}
