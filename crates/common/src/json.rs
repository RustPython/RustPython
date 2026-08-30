//! VM-independent JSON string escaping and scanning over WTF-8.

use rustpython_wtf8::{CodePoint, Wtf8, Wtf8Buf};

pub static ESCAPE_CHARS: [&str; 0x20] = [
    "\\u0000", "\\u0001", "\\u0002", "\\u0003", "\\u0004", "\\u0005", "\\u0006", "\\u0007", "\\b",
    "\\t", "\\n", "\\u000b", "\\f", "\\r", "\\u000e", "\\u000f", "\\u0010", "\\u0011", "\\u0012",
    "\\u0013", "\\u0014", "\\u0015", "\\u0016", "\\u0017", "\\u0018", "\\u0019", "\\u001a",
    "\\u001b", "\\u001c", "\\u001d", "\\u001e", "\\u001f",
];

#[must_use]
pub fn encode_string(value: &Wtf8, ascii_only: bool) -> Wtf8Buf {
    let mut out = Wtf8Buf::with_capacity(value.len() + 2);
    out.push_char('"');
    for cp in value.code_points() {
        let n = cp.to_u32();
        match n {
            0x08 => out.push_str("\\b"),
            0x09 => out.push_str("\\t"),
            0x0a => out.push_str("\\n"),
            0x0c => out.push_str("\\f"),
            0x0d => out.push_str("\\r"),
            0x00..=0x1f => out.push_str(ESCAPE_CHARS[n as usize]),
            0x22 => out.push_str("\\\""),
            0x5c => out.push_str("\\\\"),
            0x7f if ascii_only => out.push_str("\\u007f"),
            0x80.. if ascii_only => {
                if n <= 0xffff {
                    out.push_str(&format!("\\u{n:04x}"));
                } else {
                    let m = n - 0x10000;
                    let lead = 0xd800 | ((m >> 10) & 0x3ff);
                    let trail = 0xdc00 | (m & 0x3ff);
                    out.push_str(&format!("\\u{lead:04x}\\u{trail:04x}"));
                }
            }
            _ => out.push(cp),
        }
    }
    out.push_char('"');
    out
}

#[derive(Debug)]
pub struct DecodeError {
    pub msg: String,
    pub pos: usize,
}

fn decode_hex<I>(chars: &mut I, pos: usize) -> Result<CodePoint, DecodeError>
where
    I: Iterator<Item = (usize, (usize, CodePoint))>,
{
    let mut value = 0u16;
    for _ in 0..4 {
        let (_, (_, cp)) = chars.next().ok_or_else(|| DecodeError {
            msg: "Invalid \\uXXXX escape".to_owned(),
            pos,
        })?;
        let digit = cp
            .to_char()
            .and_then(|c| c.to_digit(16))
            .ok_or_else(|| DecodeError {
                msg: "Invalid \\uXXXX escape".to_owned(),
                pos,
            })?;
        value = (value << 4) | digit as u16;
    }
    Ok(value.into())
}

/// CPython `_json.scanstring`, expressed over WTF-8 so lone surrogates retain
/// their Python `str` identity rather than being lossily converted to UTF-8.
pub fn scan_string(
    value: &Wtf8,
    char_offset: usize,
    strict: bool,
) -> Result<(Wtf8Buf, usize, usize), DecodeError> {
    let unterminated = || DecodeError {
        msg: "Unterminated string starting at".to_owned(),
        pos: char_offset.saturating_sub(1),
    };
    let mut out = Wtf8Buf::new();
    let mut chars = value.code_point_indices().enumerate().peekable();
    while let Some((char_i, (byte_i, cp))) = chars.next() {
        match cp.to_char_lossy() {
            '"' => return Ok((out, char_offset + char_i + 1, byte_i + 1)),
            '\\' => {
                let (escape_i, (_, escaped)) = chars.next().ok_or_else(unterminated)?;
                match escaped.to_char_lossy() {
                    '"' => out.push_char('"'),
                    '\\' => out.push_char('\\'),
                    '/' => out.push_char('/'),
                    'b' => out.push_char('\x08'),
                    'f' => out.push_char('\x0c'),
                    'n' => out.push_char('\n'),
                    'r' => out.push_char('\r'),
                    't' => out.push_char('\t'),
                    'u' => {
                        let mut decoded = decode_hex(&mut chars, char_offset + escape_i)?;
                        if let Some(lead) = decoded.to_lead_surrogate() {
                            let mut lookahead = chars.clone();
                            if let (Some((_, (_, slash))), Some((u_i, (_, u)))) =
                                (lookahead.next(), lookahead.next())
                                && slash == '\\'
                                && u == 'u'
                            {
                                let second = decode_hex(&mut lookahead, char_offset + u_i)?;
                                if let Some(trail) = second.to_trail_surrogate() {
                                    decoded = lead.merge(trail).into();
                                    chars = lookahead;
                                }
                            }
                        }
                        out.push(decoded);
                    }
                    _ => {
                        return Err(DecodeError {
                            msg: format!("Invalid \\escape: {escaped:?}"),
                            pos: char_offset + char_i,
                        });
                    }
                }
            }
            '\x00'..='\x1f' if strict => {
                return Err(DecodeError {
                    msg: format!("Invalid control character {cp:?} at"),
                    pos: char_offset + char_i,
                });
            }
            _ => out.push(cp),
        }
    }
    Err(unterminated())
}
