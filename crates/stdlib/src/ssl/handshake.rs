use core::fmt;
use rustls::server::Acceptor;
use rustpython_vm::{
    builtins::PyBaseExceptionRef,
    object::{Traverse, TraverseFn},
};

/// Server configuration is selected only after receiving ClientHello and
/// invoking SNI. A rejected handshake stays terminal while its alert drains.
pub(super) enum HandshakeState {
    WaitingForClientHello(Acceptor),
    CallingSni,
    Handshaking,
    Connected,
    SendingAlert {
        error: PyBaseExceptionRef,
        bytes: Vec<u8>,
        sent: usize,
    },
}

impl HandshakeState {
    pub(super) fn new(server_side: bool) -> Self {
        if server_side {
            Self::WaitingForClientHello(Acceptor::default())
        } else {
            Self::Handshaking
        }
    }
}

impl fmt::Debug for HandshakeState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::WaitingForClientHello(_) => "WaitingForClientHello",
            Self::CallingSni => "CallingSni",
            Self::Handshaking => "Handshaking",
            Self::Connected => "Connected",
            Self::SendingAlert { .. } => "SendingAlert",
        })
    }
}

// Only the saved Python exception can contain GC references. Acceptor and the
// encoded TLS alert contain Rust-owned protocol data.
unsafe impl Traverse for HandshakeState {
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        if let Self::SendingAlert { error, .. } = self {
            error.traverse(tracer_fn);
        }
    }
}

/// SNI rejection happens before ServerHello, so the fatal alert is plaintext
/// even when the ClientHello offers TLS 1.3 (RFC 8446 section 5.1).
pub(super) fn sni_alert(description: u8) -> Vec<u8> {
    vec![21, 3, 3, 0, 2, 2, description]
}

pub(super) fn feed_acceptor(acceptor: &mut Acceptor, bytes: &[u8]) -> std::io::Result<()> {
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
