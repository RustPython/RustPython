use core::fmt;
use rustls::server::Acceptor;
use rustpython_vm::{
    builtins::PyBaseExceptionRef,
    object::{Traverse, TraverseFn},
};

pub(super) use rustpython_host_env::ssl::handshake::{feed_acceptor, sni_alert};

/// Server configuration is selected only after receiving ClientHello and
/// invoking SNI. A failed connection stays terminal while its alert drains.
/// ShuttingDown means our close_notify is queued and must not be sent twice.
pub(super) enum TlsState {
    WaitingForClientHello(Box<Acceptor>),
    InProgress,
    Handshaking,
    Connected,
    ShuttingDown,
    ShutDown,
    SendingAlert { error: PyBaseExceptionRef },
}

impl TlsState {
    pub(super) fn new(server_side: bool) -> Self {
        if server_side {
            Self::WaitingForClientHello(Box::default())
        } else {
            Self::Handshaking
        }
    }
}

impl fmt::Debug for TlsState {
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

// Only the saved Python exception can contain GC references. Acceptor contains
// Rust-owned protocol data; encoded alerts use the shared pending output queue.
unsafe impl Traverse for TlsState {
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        if let Self::SendingAlert { error, .. } = self {
            error.traverse(tracer_fn);
        }
    }
}
