//! `unicode-escape` encode / incremental decode.

use super::*;
use crate::wtf8::{CodePoint, Wtf8, Wtf8Buf};

pub const ENCODING_NAME: &str = "unicodeescape";

fn push_hex(out: &mut Vec<u8>, prefix: u8, cp: u32, digits: usize) {
    out.push(b'\\');
    out.push(prefix);
    for shift in (0..digits).rev() {
        out.push(b"0123456789abcdef"[((cp >> (shift * 4)) & 0xf) as usize]);
    }
}

/// Encode `s` as a Python unicode-escape byte string. Every code point is
/// representable, so this cannot fail.
#[must_use]
pub fn encode_bytes(s: &Wtf8) -> Vec<u8> {
    let mut out = Vec::new();
    for cp in s.code_points() {
        match cp.to_u32() {
            0x5c => out.extend_from_slice(br"\\"),
            0x09 => out.extend_from_slice(br"\t"),
            0x0a => out.extend_from_slice(br"\n"),
            0x0d => out.extend_from_slice(br"\r"),
            0x20..=0x7e => out.push(cp.to_u32() as u8),
            c @ 0x00..=0xff => push_hex(&mut out, b'x', c, 2),
            c @ 0x100..=0xffff => push_hex(&mut out, b'u', c, 4),
            c => push_hex(&mut out, b'U', c, 8),
        }
    }
    out
}

pub fn encode<Ctx, E>(ctx: Ctx, _errors: &E) -> Result<Vec<u8>, Ctx::Error>
where
    Ctx: EncodeContext,
    E: EncodeErrorHandler<Ctx>,
{
    Ok(encode_bytes(ctx.remaining_data()))
}

/// Optional first invalid-escape deprecation text. The caller issues it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EscapeNote {
    pub message: String,
}

fn invalid_escape_warning(prefix: &str, sequence: &str, octal: bool) -> String {
    let kind = if octal {
        "an invalid octal escape sequence"
    } else {
        "an invalid escape sequence"
    };
    format!("{prefix}\"\\{sequence}\" is {kind}. Such sequences will not work in the future. ")
}

/// Incremental unicode-escape decode.
pub fn decode<Ctx, E>(
    mut ctx: Ctx,
    errors: &E,
    final_decode: bool,
) -> Result<(Wtf8Buf, usize, Option<EscapeNote>), Ctx::Error>
where
    Ctx: DecodeContext,
    E: DecodeErrorHandler<Ctx>,
{
    let mut out = Wtf8Buf::new();
    let mut warning = None;
    loop {
        let rest = ctx.remaining_data();
        if rest.is_empty() {
            break;
        }
        let ch = rest[0];
        if ch != b'\\' {
            if let Some(cp) = CodePoint::from_u32(ch as u32) {
                out.push(cp);
            }
            ctx.advance(1);
            continue;
        }
        let escape_start = ctx.position();
        if rest.len() == 1 {
            if !final_decode {
                break;
            }
            let end = ctx.full_data().len();
            let replace =
                ctx.handle_error(errors, escape_start..end, Some("\\ at end of string"))?;
            out.push_wtf8(replace.as_ref());
            continue;
        }
        let intro = rest[1];
        match intro {
            b'\n' => ctx.advance(2),
            b'\\' => {
                out.push_char('\\');
                ctx.advance(2);
            }
            b'\'' => {
                out.push_char('\'');
                ctx.advance(2);
            }
            b'"' => {
                out.push_char('"');
                ctx.advance(2);
            }
            b'b' => {
                out.push_char('\x08');
                ctx.advance(2);
            }
            b'f' => {
                out.push_char('\x0c');
                ctx.advance(2);
            }
            b't' => {
                out.push_char('\t');
                ctx.advance(2);
            }
            b'n' => {
                out.push_char('\n');
                ctx.advance(2);
            }
            b'r' => {
                out.push_char('\r');
                ctx.advance(2);
            }
            b'v' => {
                out.push_char('\x0b');
                ctx.advance(2);
            }
            b'a' => {
                out.push_char('\x07');
                ctx.advance(2);
            }
            b'0'..=b'7' => {
                ctx.advance(2);
                let mut value = (intro - b'0') as u32;
                let octal_start = escape_start + 1;
                for _ in 0..2 {
                    let rest = ctx.remaining_data();
                    if rest.first().is_some_and(|b| matches!(b, b'0'..=b'7')) {
                        value = (value << 3) + (rest[0] - b'0') as u32;
                        ctx.advance(1);
                    }
                }
                if value > 0o377 && warning.is_none() {
                    let seq = &ctx.full_data()[octal_start..ctx.position()];
                    warning = Some(EscapeNote {
                        message: invalid_escape_warning("", &String::from_utf8_lossy(seq), true),
                    });
                }
                if let Some(cp) = CodePoint::from_u32(value) {
                    out.push(cp);
                }
            }
            b'x' | b'u' | b'U' => {
                let (digits, message) = match intro {
                    b'x' => (2usize, "truncated \\xXX escape"),
                    b'u' => (4usize, "truncated \\uXXXX escape"),
                    _ => (8usize, "truncated \\UXXXXXXXX escape"),
                };
                if !final_decode && rest.len() < 2 + digits {
                    break;
                }
                ctx.advance(2);
                let rest = ctx.remaining_data();
                if rest.len() >= digits && rest[..digits].iter().all(u8::is_ascii_hexdigit) {
                    let value =
                        u32::from_str_radix(core::str::from_utf8(&rest[..digits]).unwrap(), 16)
                            .unwrap();
                    if let Some(cp) = CodePoint::from_u32(value) {
                        out.push(cp);
                        ctx.advance(digits);
                        continue;
                    }
                    let start = escape_start;
                    let end = ctx.position() + digits;
                    let replace =
                        ctx.handle_error(errors, start..end, Some("illegal Unicode character"))?;
                    out.push_wtf8(replace.as_ref());
                    continue;
                }
                let mut hex_end = 0;
                while hex_end < rest.len() && rest[hex_end].is_ascii_hexdigit() {
                    hex_end += 1;
                }
                let start = escape_start;
                let end = ctx.position() + hex_end;
                let replace = ctx.handle_error(errors, start..end, Some(message))?;
                out.push_wtf8(replace.as_ref());
            }
            b'N' => {
                if rest.len() == 2 && !final_decode {
                    break;
                }
                if rest.len() >= 3 && rest[2] == b'{' {
                    let name_start = 3;
                    let mut look = name_start;
                    while look < rest.len() && rest[look] != b'}' {
                        look += 1;
                    }
                    if look >= rest.len() {
                        if !final_decode {
                            break;
                        }
                        let start = escape_start;
                        let end = ctx.full_data().len();
                        let replace = ctx.handle_error(
                            errors,
                            start..end,
                            Some("malformed \\N character escape"),
                        )?;
                        out.push_wtf8(replace.as_ref());
                        continue;
                    }
                    if look == name_start {
                        let start = escape_start;
                        let end = ctx.position() + name_start;
                        let replace = ctx.handle_error(
                            errors,
                            start..end,
                            Some("malformed \\N character escape"),
                        )?;
                        out.push_wtf8(replace.as_ref());
                        continue;
                    }
                    let name = core::str::from_utf8(&rest[name_start..look]).ok();
                    match name.and_then(rustpython_unicode::lookup_character) {
                        Some(ch) => {
                            out.push_char(ch);
                            ctx.advance(look + 1);
                            continue;
                        }
                        None => {
                            let start = escape_start;
                            let end = ctx.position() + look + 1;
                            let replace = ctx.handle_error(
                                errors,
                                start..end,
                                Some("unknown Unicode character name"),
                            )?;
                            out.push_wtf8(replace.as_ref());
                            continue;
                        }
                    }
                }
                ctx.advance(2);
                let start = escape_start;
                let end = ctx.position();
                let replace =
                    ctx.handle_error(errors, start..end, Some("malformed \\N character escape"))?;
                out.push_wtf8(replace.as_ref());
            }
            other => {
                if warning.is_none() {
                    warning = Some(EscapeNote {
                        message: invalid_escape_warning("", &char::from(other).to_string(), false),
                    });
                }
                out.push_char('\\');
                if let Some(cp) = CodePoint::from_u32(other as u32) {
                    out.push(cp);
                }
                ctx.advance(2);
            }
        }
    }
    Ok((out, ctx.position(), warning))
}
