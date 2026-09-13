//! TLS record observation for `_msg_callback` and tls-unique.

use rustls::SupportedCipherSuite;

pub const SSL3_RT_CHANGE_CIPHER_SPEC: i32 = 20;
pub const SSL3_RT_ALERT: i32 = 21;
pub const SSL3_RT_HANDSHAKE: i32 = 22;
pub const SSL3_RT_APPLICATION_DATA: u8 = 23;
pub const SSL3_RT_HEADER: i32 = 256;
pub const SSL3_MT_CHANGE_CIPHER_SPEC: i32 = 0x0101;
const TLS_RECORD_HEADER_SIZE: usize = 5;

/// Per-connection assembler for TLS records and handshake messages.
#[derive(Debug, Default)]
pub struct MsgState {
    incoming_record: Vec<u8>,
    outgoing_record: Vec<u8>,
    incoming_hs: Vec<u8>,
    outgoing_hs: Vec<u8>,
    transcript: Vec<u8>,
    saw_ccs: bool,
}

/// One `_msg_callback` event reconstructed from a TLS record.
#[derive(Debug)]
pub struct MsgEvent {
    pub version: i32,
    pub content_type: i32,
    pub msg_type: i32,
    pub data: Vec<u8>,
}

impl MsgState {
    /// Handshake bytes observed before ChangeCipherSpec.
    #[must_use]
    pub fn transcript(&self) -> &[u8] {
        &self.transcript
    }

    /// Consume ciphertext and return the records and handshake messages it
    /// completed.
    pub fn observe(&mut self, write: bool, bytes: &[u8]) -> Vec<MsgEvent> {
        let (record_buf, hs_buf) = if write {
            (&mut self.outgoing_record, &mut self.outgoing_hs)
        } else {
            (&mut self.incoming_record, &mut self.incoming_hs)
        };
        record_buf.extend_from_slice(bytes);
        let mut events = Vec::new();
        for (content_type, version, payload) in take_records(record_buf) {
            let mut header = Vec::with_capacity(TLS_RECORD_HEADER_SIZE);
            header.push(content_type);
            header.extend_from_slice(&version.to_be_bytes());
            header.extend_from_slice(&(payload.len() as u16).to_be_bytes());
            events.push(MsgEvent {
                version: version as i32,
                content_type: SSL3_RT_HEADER,
                msg_type: content_type as i32,
                data: header,
            });
            match content_type {
                x if x == SSL3_RT_CHANGE_CIPHER_SPEC as u8 => {
                    events.push(MsgEvent {
                        version: version as i32,
                        content_type: SSL3_RT_CHANGE_CIPHER_SPEC,
                        msg_type: SSL3_MT_CHANGE_CIPHER_SPEC,
                        data: payload,
                    });
                    hs_buf.clear();
                    self.saw_ccs = true;
                }
                x if x == SSL3_RT_ALERT as u8 => {
                    let msg_type = payload.get(1).copied().map_or(-1, |b| b as i32);
                    events.push(MsgEvent {
                        version: version as i32,
                        content_type: SSL3_RT_ALERT,
                        msg_type,
                        data: payload,
                    });
                }
                x if x == SSL3_RT_HANDSHAKE as u8 => {
                    if self.saw_ccs {
                        continue;
                    }
                    hs_buf.extend_from_slice(&payload);
                    for message in take_handshake_messages(hs_buf) {
                        self.transcript.extend_from_slice(&message);
                        let msg_type = message.first().copied().map_or(-1, |b| b as i32);
                        events.push(MsgEvent {
                            version: version as i32,
                            content_type: SSL3_RT_HANDSHAKE,
                            msg_type,
                            data: message,
                        });
                    }
                }
                SSL3_RT_APPLICATION_DATA => {}
                _ => events.push(MsgEvent {
                    version: version as i32,
                    content_type: content_type as i32,
                    msg_type: -1,
                    data: payload,
                }),
            }
        }
        events
    }
}

fn take_records(buf: &mut Vec<u8>) -> Vec<(u8, u16, Vec<u8>)> {
    let mut records = Vec::new();
    loop {
        if buf.len() < TLS_RECORD_HEADER_SIZE {
            break;
        }
        let len = u16::from_be_bytes([buf[3], buf[4]]) as usize;
        if buf.len() < TLS_RECORD_HEADER_SIZE + len {
            break;
        }
        let content_type = buf[0];
        let version = u16::from_be_bytes([buf[1], buf[2]]);
        let payload = buf[TLS_RECORD_HEADER_SIZE..TLS_RECORD_HEADER_SIZE + len].to_vec();
        buf.drain(..TLS_RECORD_HEADER_SIZE + len);
        records.push((content_type, version, payload));
    }
    records
}

fn take_handshake_messages(buf: &mut Vec<u8>) -> Vec<Vec<u8>> {
    let mut messages = Vec::new();
    loop {
        if buf.len() < 4 {
            break;
        }
        let len = u32::from_be_bytes([0, buf[1], buf[2], buf[3]]) as usize;
        if buf.len() < 4 + len {
            break;
        }
        messages.push(buf[..4 + len].to_vec());
        buf.drain(..4 + len);
    }
    messages
}

/// TLS 1.2 `tls-unique` binding from the master secret and handshake transcript.
#[must_use]
pub fn tls12_unique(
    suite: SupportedCipherSuite,
    master_secret: &[u8],
    transcript: &[u8],
    server_side: bool,
    session_reused: bool,
) -> Option<Vec<u8>> {
    let SupportedCipherSuite::Tls12(suite) = suite else {
        return None;
    };
    if master_secret.is_empty() || transcript.is_empty() {
        return None;
    }
    let hash = suite.common.hash_provider.hash(transcript);
    let is_client = !server_side;
    let ours = session_reused ^ is_client;
    let label: &[u8] = match (ours, server_side) {
        (true, false) | (false, true) => b"client finished",
        (true, true) | (false, false) => b"server finished",
    };
    let mut out = vec![0u8; 12];
    suite
        .prf_provider
        .for_secret(&mut out, master_secret, label, hash.as_ref());
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_complete_records_and_handshake_messages() {
        let mut state = MsgState::default();
        let mut record = vec![22, 3, 3, 0, 8];
        record.extend_from_slice(&[12, 0, 0, 4, 1, 2, 3, 4]);
        let events = state.observe(false, &record);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].content_type, SSL3_RT_HEADER);
        assert_eq!(events[1].content_type, SSL3_RT_HANDSHAKE);
        assert_eq!(events[1].msg_type, 12);
        assert_eq!(state.transcript(), [12, 0, 0, 4, 1, 2, 3, 4]);
    }

    #[test]
    fn ccs_stops_transcript() {
        let mut state = MsgState::default();
        let mut hs = vec![22, 3, 3, 0, 4];
        hs.extend_from_slice(&[1, 0, 0, 0]);
        state.observe(true, &hs);
        state.observe(true, &[20, 3, 3, 0, 1, 1]);
        let before = state.transcript().to_vec();
        let mut finished = vec![22, 3, 3, 0, 4];
        finished.extend_from_slice(&[20, 0, 0, 0]);
        state.observe(true, &finished);
        assert_eq!(state.transcript(), before);
    }
}
