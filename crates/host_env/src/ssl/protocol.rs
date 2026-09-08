//! Protocol version, ALPN, and hostname helpers.

use rustls::version::{TLS12, TLS13};

use super::constants::{OP_NO_TLSV1_2, OP_NO_TLSV1_3, PROTO_TLSV1_2, PROTO_TLSV1_3};

/// Why `server_hostname` was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostnameError {
    Empty,
    LeadingDot,
    EmbeddedNul,
    TooLong,
}

impl HostnameError {
    #[must_use]
    pub fn message(&self) -> &'static str {
        match self {
            Self::Empty => "server_hostname cannot be an empty string",
            Self::LeadingDot => "server_hostname cannot start with a dot",
            Self::EmbeddedNul => "embedded null character",
            Self::TooLong => "server_hostname is too long (maximum 253 characters)",
        }
    }
}

/// Validate `server_hostname` the way `_SSLSocket` does.
pub fn validate_hostname(hostname: &str) -> Result<(), HostnameError> {
    if hostname.is_empty() {
        return Err(HostnameError::Empty);
    }
    if hostname.starts_with('.') {
        return Err(HostnameError::LeadingDot);
    }
    if hostname.as_bytes().contains(&0) {
        return Err(HostnameError::EmbeddedNul);
    }
    if hostname.len() > 253 {
        return Err(HostnameError::TooLong);
    }
    Ok(())
}

/// Convert PROTO/OP bits into the rustls version slice.
#[must_use]
pub fn rustls_versions(
    minimum: i32,
    maximum: i32,
    options: i32,
) -> &'static [&'static rustls::SupportedProtocolVersion] {
    static TLS12_ONLY: &[&rustls::SupportedProtocolVersion] = &[&TLS12];
    static TLS13_ONLY: &[&rustls::SupportedProtocolVersion] = &[&TLS13];

    let min = if minimum == -2 {
        PROTO_TLSV1_2
    } else {
        minimum
    };
    let max = if maximum == -1 {
        PROTO_TLSV1_3
    } else {
        maximum
    };

    let tls12_disabled = options & OP_NO_TLSV1_2 != 0;
    let tls13_disabled = options & OP_NO_TLSV1_3 != 0;
    let want_tls12 =
        (min == 0 || min <= PROTO_TLSV1_2) && (max == 0 || max >= PROTO_TLSV1_2) && !tls12_disabled;
    let want_tls13 =
        (min == 0 || min <= PROTO_TLSV1_3) && (max == 0 || max >= PROTO_TLSV1_3) && !tls13_disabled;

    match (want_tls12, want_tls13) {
        (true, true) | (false, false) => rustls::DEFAULT_VERSIONS,
        (true, false) => TLS12_ONLY,
        (false, true) => TLS13_ONLY,
    }
}

/// Why a length-prefixed ALPN list was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlpnError(pub String);

/// Parse `[len][proto]...` the way `ssl.py` hands it to `_set_alpn_protocols`.
pub fn parse_length_prefixed_alpn(bytes: &[u8]) -> Result<Vec<Vec<u8>>, AlpnError> {
    let mut alpn_list = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        if offset + 1 > bytes.len() {
            return Err(AlpnError(format!(
                "Invalid ALPN protocol data: unexpected end at offset {offset}"
            )));
        }
        let proto_len = bytes[offset] as usize;
        offset += 1;
        if proto_len == 0 {
            return Err(AlpnError(format!(
                "Invalid ALPN protocol data: protocol length cannot be 0 at offset {}",
                offset - 1
            )));
        }
        if offset + proto_len > bytes.len() {
            return Err(AlpnError(format!(
                "Invalid ALPN protocol data: expected {} bytes at offset {}, but only {} bytes remain",
                proto_len,
                offset,
                bytes.len() - offset
            )));
        }
        alpn_list.push(bytes[offset..offset + proto_len].to_vec());
        offset += proto_len;
    }
    Ok(alpn_list)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostname_rejects_empty_dot_nul_and_overlong() {
        assert_eq!(validate_hostname(""), Err(HostnameError::Empty));
        assert_eq!(
            validate_hostname(".example"),
            Err(HostnameError::LeadingDot)
        );
        assert_eq!(
            validate_hostname("ex\0ample"),
            Err(HostnameError::EmbeddedNul)
        );
        assert_eq!(
            validate_hostname(&"a".repeat(254)),
            Err(HostnameError::TooLong)
        );
        assert!(validate_hostname("localhost").is_ok());
    }

    #[test]
    fn alpn_reads_length_prefixed_protocols() {
        assert_eq!(
            parse_length_prefixed_alpn(&[
                2, b'h', b'2', 8, b'h', b't', b't', b'p', b'/', b'1', b'.', b'1'
            ])
            .unwrap(),
            [b"h2".to_vec(), b"http/1.1".to_vec()]
        );
        assert!(parse_length_prefixed_alpn(&[0]).is_err());
        assert!(parse_length_prefixed_alpn(&[3, b'h', b'2']).is_err());
    }
}
