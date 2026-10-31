//! rustls client session store plus synthetic session metadata.

use alloc::sync::Arc;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::{collections::HashMap, time::SystemTime};

use parking_lot::{Mutex, RwLock};
use rustls::{
    NamedGroup,
    client::{
        ClientSessionMemoryCache, ClientSessionStore, Tls12ClientSessionValue,
        Tls13ClientSessionValue,
    },
    pki_types::ServerName,
};
use sha2::{Digest, Sha256};

pub const SESSION_CACHE_SIZE: usize = 256;
static NEXT_SESSION_NONCE: AtomicUsize = AtomicUsize::new(1);

/// Synthetic session metadata. rustls does not expose ticket bytes.
#[derive(Debug, Clone)]
pub struct SessionData {
    pub server_name: String,
    pub session_id: Vec<u8>,
    pub creation_time: SystemTime,
    pub lifetime: u64,
}

impl SessionData {
    #[must_use]
    pub fn new(server_name: &str, lifetime: u64) -> Self {
        let creation_time = SystemTime::now();
        let nonce = NEXT_SESSION_NONCE.fetch_add(1, Ordering::Relaxed);
        let mut hasher = Sha256::new();
        hasher.update(server_name.as_bytes());
        hasher.update(
            creation_time
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
                .to_le_bytes(),
        );
        hasher.update(nonce.to_le_bytes());
        Self {
            server_name: server_name.to_owned(),
            session_id: hasher.finalize()[..16].to_vec(),
            creation_time,
            lifetime,
        }
    }
}

pub type SessionCache = Arc<RwLock<HashMap<Vec<u8>, Arc<Mutex<SessionData>>>>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientSessionKind {
    Tls12,
    Tls13,
}

/// rustls session store that also records metadata for `SSLSession`.
#[derive(Debug)]
pub struct CapturingClientSessionStore {
    inner: Arc<ClientSessionMemoryCache>,
    session_cache: SessionCache,
}

impl CapturingClientSessionStore {
    #[must_use]
    pub fn new(session_cache: SessionCache) -> Self {
        Self {
            inner: Arc::new(ClientSessionMemoryCache::new(SESSION_CACHE_SIZE)),
            session_cache,
        }
    }

    pub fn transfer_session(
        &self,
        target: &Self,
        server_name: &ServerName<'static>,
        kind: ClientSessionKind,
    ) {
        if let Some(group) = self.kx_hint(server_name) {
            target.set_kx_hint(server_name.clone(), group);
        }
        match kind {
            ClientSessionKind::Tls12 => {
                if let Some(session) = self.tls12_session(server_name) {
                    target.set_tls12_session(server_name.clone(), session);
                }
            }
            ClientSessionKind::Tls13 => {
                if let Some(ticket) = self.take_tls13_ticket(server_name) {
                    target.insert_tls13_ticket(server_name.clone(), ticket);
                }
            }
        }
    }

    fn record_metadata(&self, server_name: &ServerName<'static>) {
        let server_name_str = server_name.to_str();
        let session_data = SessionData::new(server_name_str.as_ref(), 7200);
        self.session_cache.write().insert(
            server_name_str.as_bytes().to_vec(),
            Arc::new(Mutex::new(session_data)),
        );
    }
}

impl ClientSessionStore for CapturingClientSessionStore {
    fn set_kx_hint(&self, server_name: ServerName<'static>, group: NamedGroup) {
        self.inner.set_kx_hint(server_name, group);
    }

    fn kx_hint(&self, server_name: &ServerName<'_>) -> Option<NamedGroup> {
        self.inner.kx_hint(server_name)
    }

    fn set_tls12_session(&self, server_name: ServerName<'static>, value: Tls12ClientSessionValue) {
        self.inner.set_tls12_session(server_name.clone(), value);
        self.record_metadata(&server_name);
    }

    fn tls12_session(&self, server_name: &ServerName<'_>) -> Option<Tls12ClientSessionValue> {
        self.inner.tls12_session(server_name)
    }

    fn remove_tls12_session(&self, server_name: &ServerName<'static>) {
        self.inner.remove_tls12_session(server_name);
        self.session_cache
            .write()
            .remove(server_name.to_str().as_bytes());
    }

    fn insert_tls13_ticket(
        &self,
        server_name: ServerName<'static>,
        value: Tls13ClientSessionValue,
    ) {
        self.inner.insert_tls13_ticket(server_name.clone(), value);
        self.record_metadata(&server_name);
    }

    fn take_tls13_ticket(
        &self,
        server_name: &ServerName<'static>,
    ) -> Option<Tls13ClientSessionValue> {
        self.inner.take_tls13_ticket(server_name)
    }
}
