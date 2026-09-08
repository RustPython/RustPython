//! Sans-I/O rustls connection plus one-record transport helpers.

use std::io::{Read, Write};

use rustls::{
    Connection, HandshakeKind, IoState, ProtocolVersion, SupportedCipherSuite,
    pki_types::CertificateDer,
};

use super::error::TlsError;

pub const TLS_RECORD_HEADER_SIZE: usize = 5;
const SSL3_RT_MAX_MD_SIZE: usize = 64;
const SSL3_RT_MAX_PLAIN_LENGTH: usize = 16384;
const SSL3_RT_MAX_COMPRESSED_OVERHEAD: usize = 1024;
const SSL3_RT_MAX_ENCRYPTED_OVERHEAD: usize = 256 + SSL3_RT_MAX_MD_SIZE;
const SSL3_RT_MAX_COMPRESSED_LENGTH: usize =
    SSL3_RT_MAX_PLAIN_LENGTH + SSL3_RT_MAX_COMPRESSED_OVERHEAD;
const SSL3_RT_MAX_ENCRYPTED_LENGTH: usize =
    SSL3_RT_MAX_ENCRYPTED_OVERHEAD + SSL3_RT_MAX_COMPRESSED_LENGTH;
pub const SSL3_RT_MAX_PACKET_SIZE: usize = SSL3_RT_MAX_ENCRYPTED_LENGTH + TLS_RECORD_HEADER_SIZE;

/// One TLS record's header and remaining body, so a transport read never
/// lifts more than the record currently being parsed.
#[derive(Debug, Default)]
pub struct RecordCursor {
    header: [u8; TLS_RECORD_HEADER_SIZE],
    header_read: usize,
    body_left: usize,
}

impl RecordCursor {
    /// Bytes still owed before the next record boundary.
    #[must_use]
    pub fn want(&self) -> usize {
        if self.body_left > 0 {
            self.body_left
        } else {
            TLS_RECORD_HEADER_SIZE - self.header_read
        }
    }

    /// True while the 5-byte header is still incomplete, or after a
    /// zero-length body (the next read starts a new header).
    #[must_use]
    pub fn in_header(&self) -> bool {
        self.body_left == 0
    }

    /// Account for `data`, which must be no longer than [`want`].
    pub fn consume(&mut self, data: &[u8]) {
        if self.body_left > 0 {
            self.body_left -= data.len();
            return;
        }
        self.header[self.header_read..self.header_read + data.len()].copy_from_slice(data);
        self.header_read += data.len();
        if self.header_read == TLS_RECORD_HEADER_SIZE {
            self.body_left = u16::from_be_bytes([self.header[3], self.header[4]]) as usize;
            self.header_read = 0;
        }
    }
}

/// rustls connection plus write-retry state. Unsent TLS stays on the
/// interpreter object so the rustls lock is not held across socket I/O.
#[derive(Debug)]
pub struct TlsConnection {
    inner: Connection,
    write_buffered_len: usize,
}

impl TlsConnection {
    #[must_use]
    pub fn new(inner: Connection) -> Self {
        Self {
            inner,
            write_buffered_len: 0,
        }
    }

    #[must_use]
    pub fn inner(&self) -> &Connection {
        &self.inner
    }

    pub fn inner_mut(&mut self) -> &mut Connection {
        &mut self.inner
    }

    #[must_use]
    pub fn wants_read(&self) -> bool {
        self.inner.wants_read()
    }

    #[must_use]
    pub fn wants_write(&self) -> bool {
        self.inner.wants_write()
    }

    #[must_use]
    pub fn is_handshaking(&self) -> bool {
        self.inner.is_handshaking()
    }

    #[must_use]
    pub fn protocol_version(&self) -> Option<ProtocolVersion> {
        self.inner.protocol_version()
    }

    #[must_use]
    pub fn negotiated_cipher_suite(&self) -> Option<SupportedCipherSuite> {
        self.inner.negotiated_cipher_suite()
    }

    #[must_use]
    pub fn alpn_protocol(&self) -> Option<&[u8]> {
        self.inner.alpn_protocol()
    }

    #[must_use]
    pub fn peer_certificates(&self) -> Option<&[CertificateDer<'static>]> {
        self.inner.peer_certificates()
    }

    #[must_use]
    pub fn handshake_kind(&self) -> Option<HandshakeKind> {
        self.inner.handshake_kind()
    }

    #[must_use]
    pub fn write_buffered_len(&self) -> usize {
        self.write_buffered_len
    }

    pub fn set_write_buffered_len(&mut self, n: usize) {
        self.write_buffered_len = n;
    }

    #[must_use]
    pub fn pending_plaintext(&mut self) -> usize {
        use std::io::BufRead;
        self.inner.reader().fill_buf().map_or(0, |buf| buf.len())
    }

    pub fn send_close_notify(&mut self) {
        self.inner.send_close_notify();
    }

    pub fn process_packets(&mut self) -> Result<IoState, TlsError> {
        self.inner
            .process_new_packets()
            .map_err(TlsError::from_rustls)
    }

    pub fn peer_has_closed(&mut self) -> Result<bool, TlsError> {
        Ok(self.process_packets()?.peer_has_closed())
    }

    /// Drain every record rustls has queued for the peer.
    pub fn drain_tls(&mut self) -> Result<Vec<u8>, TlsError> {
        let mut buf = Vec::new();
        let n = self.inner.write_tls(&mut buf).map_err(TlsError::Io)?;
        if n > 0 { Ok(buf) } else { Ok(Vec::new()) }
    }

    /// Feed ciphertext. An empty slice means the transport returned EOF.
    pub fn feed_tls(&mut self, data: &[u8]) -> Result<(), TlsError> {
        if data.is_empty() {
            return if self.peer_has_closed()? {
                Err(TlsError::ZeroReturn)
            } else {
                Err(TlsError::Eof)
            };
        }

        let mut offset = 0;
        while offset < data.len() {
            let remaining = &data[offset..];
            let mut cursor = std::io::Cursor::new(remaining);
            match self.inner.read_tls(&mut cursor) {
                Ok(0) => {
                    self.process_packets()?;
                    let mut retry = std::io::Cursor::new(remaining);
                    match self.inner.read_tls(&mut retry) {
                        Ok(0) => break,
                        Ok(n) => {
                            offset += n;
                            if offset < data.len() {
                                self.process_packets()?;
                            }
                        }
                        Err(error) => return Err(TlsError::Io(error)),
                    }
                }
                Ok(read) => {
                    offset += read;
                    if offset < data.len() {
                        self.process_packets()?;
                    }
                }
                Err(error) => return Err(TlsError::Io(error)),
            }
        }
        Ok(())
    }

    /// Read application data. `None` means rustls has no plaintext yet.
    pub fn read_plaintext(&mut self, buf: &mut [u8]) -> Result<Option<usize>, TlsError> {
        match self.inner.reader().read(buf) {
            Ok(n) => Ok(Some(n)),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(TlsError::Io(error)),
        }
    }

    /// Write application data. `is_bio` turns a full buffer into `WantWrite`.
    pub fn write_plaintext(&mut self, data: &[u8], is_bio: bool) -> Result<usize, TlsError> {
        match self.inner.writer().write(data) {
            Ok(0) if !data.is_empty() => {
                if is_bio {
                    Err(TlsError::WantWrite)
                } else {
                    Err(TlsError::Syscall("Write failed: buffer full".to_string()))
                }
            }
            Ok(n) => Ok(n),
            Err(error) => {
                if is_bio {
                    Err(TlsError::WantWrite)
                } else {
                    Err(TlsError::Syscall(format!("Write failed: {error}")))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_cursor_splits_header_and_body() {
        let mut cursor = RecordCursor::default();
        assert_eq!(cursor.want(), 5);
        cursor.consume(&[22, 3, 3]);
        assert_eq!(cursor.want(), 2);
        cursor.consume(&[0, 4]);
        assert_eq!(cursor.want(), 4);
        cursor.consume(&[1, 2, 3, 4]);
        assert_eq!(cursor.want(), 5);
    }
}
