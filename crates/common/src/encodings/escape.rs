//! Bytes-to-bytes Python string-literal escape transform (`escape_encode` /
//! `escape_decode`).

use alloc::string::String;
use alloc::vec::Vec;

/// Encode `data` the way `escape_encode` / `string_escape_encode` does.
#[must_use]
pub fn encode(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    for &byte in data {
        match byte {
            b'\t' => out.extend_from_slice(b"\\t"),
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\r' => out.extend_from_slice(b"\\r"),
            b'\\' => out.extend_from_slice(b"\\\\"),
            b'\'' => out.extend_from_slice(b"\\'"),
            0x20..=0x7e => out.push(byte),
            value => out.extend_from_slice(format!("\\x{value:02x}").as_bytes()),
        }
    }
    out
}

/// How a malformed `\x` escape is reported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EscapeErrorMode {
    Strict,
    Replace,
    Ignore,
}

impl EscapeErrorMode {
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "strict" => Some(Self::Strict),
            "replace" => Some(Self::Replace),
            "ignore" => Some(Self::Ignore),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EscapeDecodeError {
    TrailingBackslash,
    InvalidHex { position: usize },
    UnknownHandler { name: String },
}

/// Decode a Python bytes-literal escape sequence.
///
/// `warning` is the first invalid-escape deprecation text, if any.
pub fn decode(
    data: &[u8],
    errors: EscapeErrorMode,
) -> Result<(Vec<u8>, Option<String>), EscapeDecodeError> {
    let mut out = Vec::with_capacity(data.len());
    let mut pos = 0usize;
    let mut warning = None;
    while pos < data.len() {
        if data[pos] != b'\\' {
            out.push(data[pos]);
            pos += 1;
            continue;
        }
        let escape_start = pos;
        pos += 1;
        if pos == data.len() {
            return Err(EscapeDecodeError::TrailingBackslash);
        }
        let ch = data[pos];
        pos += 1;
        match ch {
            b'\n' => {}
            b'\\' => out.push(b'\\'),
            b'\'' => out.push(b'\''),
            b'"' => out.push(b'"'),
            b'b' => out.push(0x08),
            b'f' => out.push(0x0c),
            b't' => out.push(b'\t'),
            b'n' => out.push(b'\n'),
            b'r' => out.push(b'\r'),
            b'v' => out.push(0x0b),
            b'a' => out.push(0x07),
            b'0'..=b'7' => {
                let octal_start = pos - 1;
                while pos < data.len()
                    && pos < octal_start + 3
                    && (b'0'..=b'7').contains(&data[pos])
                {
                    pos += 1;
                }
                let raw = data[octal_start..pos]
                    .iter()
                    .fold(0u16, |value, digit| value * 8 + u16::from(digit - b'0'));
                if raw >= 256 && warning.is_none() {
                    warning = Some(invalid_escape_warning(
                        "b",
                        &String::from_utf8_lossy(&data[octal_start..pos]),
                        true,
                    ));
                }
                out.push(raw as u8);
            }
            b'x' => {
                let hi = data.get(pos).and_then(|byte| (*byte as char).to_digit(16));
                let lo = data
                    .get(pos + 1)
                    .and_then(|byte| (*byte as char).to_digit(16));
                if let (Some(hi), Some(lo)) = (hi, lo) {
                    out.push((hi * 16 + lo) as u8);
                    pos += 2;
                } else {
                    match errors {
                        EscapeErrorMode::Strict => {
                            return Err(EscapeDecodeError::InvalidHex {
                                position: escape_start,
                            });
                        }
                        EscapeErrorMode::Replace => out.push(b'?'),
                        EscapeErrorMode::Ignore => {}
                    }
                    if data.get(pos).is_some_and(u8::is_ascii_hexdigit) {
                        pos += 1;
                    }
                }
            }
            other => {
                out.push(b'\\');
                pos -= 1;
                if warning.is_none() {
                    warning = Some(invalid_escape_warning(
                        "b",
                        &char::from(other).to_string(),
                        false,
                    ));
                }
            }
        }
    }
    Ok((out, warning))
}

fn invalid_escape_warning(prefix: &str, sequence: &str, octal: bool) -> String {
    let kind = if octal {
        "an invalid octal escape sequence"
    } else {
        "an invalid escape sequence"
    };
    format!("{prefix}\"\\{sequence}\" is {kind}. Such sequences will not work in the future. ")
}
