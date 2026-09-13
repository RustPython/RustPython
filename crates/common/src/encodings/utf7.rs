//! UTF-7 encode / incremental decode.

use super::*;
use crate::wtf8::{CodePoint, Wtf8, Wtf8Buf};

pub const ENCODING_NAME: &str = "utf-7";

fn is_base64(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'+' || b == b'/'
}

fn to_base64(n: u32) -> u8 {
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"[(n & 0x3f) as usize]
}

fn from_base64(b: u8) -> u32 {
    match b {
        b'a'..=b'z' => (b - 71) as u32,
        b'A'..=b'Z' => (b - 65) as u32,
        b'0'..=b'9' => (b + 4) as u32,
        b'+' => 62,
        _ => 63,
    }
}

fn decode_direct(b: u8) -> bool {
    b <= 127 && b != b'+'
}

fn category(oc: u32) -> u8 {
    if oc > 127 {
        return 3;
    }
    let b = oc as u8;
    if matches!(b, b'\t' | b'\n' | b'\r' | b' ') {
        2
    } else if b.is_ascii_alphanumeric() || b"'(),-./:?".contains(&b) {
        0
    } else if b"!\"#$%&*;<=>@[]^_`{|}".contains(&b) {
        1
    } else {
        3
    }
}

fn encode_direct(oc: u32) -> bool {
    oc < 128 && oc > 0 && category(oc) != 3
}

fn encode_unit(out: &mut Vec<u8>, unit: u32, bits: &mut u32, buffer: &mut u32) {
    *bits += 16;
    *buffer = (*buffer << 16) | unit;
    while *bits >= 6 {
        out.push(to_base64(*buffer >> (*bits - 6)));
        *bits -= 6;
    }
    *buffer &= (1 << *bits) - 1;
}

/// Encode `s` as UTF-7. Every code point is representable, so this cannot fail.
#[must_use]
pub fn encode_bytes(s: &Wtf8) -> Vec<u8> {
    let mut out = Vec::new();
    let mut in_shift = false;
    let mut bits = 0;
    let mut buffer = 0;
    for cp in s.code_points() {
        let oc = cp.to_u32();
        if !in_shift {
            if oc == b'+' as u32 {
                out.extend_from_slice(b"+-");
            } else if encode_direct(oc) {
                out.push(oc as u8);
            } else {
                out.push(b'+');
                in_shift = true;
                emit_code(&mut out, oc, &mut bits, &mut buffer);
            }
        } else if encode_direct(oc) {
            if bits != 0 {
                out.push(to_base64(buffer << (6 - bits)));
                buffer = 0;
                bits = 0;
            }
            in_shift = false;
            if is_base64(oc as u8) || oc == b'-' as u32 {
                out.push(b'-');
            }
            out.push(oc as u8);
        } else {
            emit_code(&mut out, oc, &mut bits, &mut buffer);
        }
    }
    if bits != 0 {
        out.push(to_base64(buffer << (6 - bits)));
    }
    if in_shift {
        out.push(b'-');
    }
    out
}

fn emit_code(out: &mut Vec<u8>, oc: u32, bits: &mut u32, buffer: &mut u32) {
    if oc >= 0x10000 {
        encode_unit(out, 0xd800 | ((oc - 0x10000) >> 10), bits, buffer);
        encode_unit(out, 0xdc00 | ((oc - 0x10000) & 0x3ff), bits, buffer);
    } else {
        encode_unit(out, oc, bits, buffer);
    }
}

pub fn encode<Ctx, E>(ctx: Ctx, _errors: &E) -> Result<Vec<u8>, Ctx::Error>
where
    Ctx: EncodeContext,
    E: EncodeErrorHandler<Ctx>,
{
    Ok(encode_bytes(ctx.remaining_data()))
}

/// Incremental UTF-7 decode.
pub fn decode<Ctx, E>(
    mut ctx: Ctx,
    errors: &E,
    final_decode: bool,
) -> Result<(Wtf8Buf, usize), Ctx::Error>
where
    Ctx: DecodeContext,
    E: DecodeErrorHandler<Ctx>,
{
    let mut out = Wtf8Buf::new();
    let mut in_shift = false;
    let mut bits = 0u32;
    let mut buffer = 0u32;
    let mut surrogate = 0u32;
    let mut shift_out_start = 0usize;
    let mut startinpos = 0usize;
    loop {
        let rest = ctx.remaining_data();
        if rest.is_empty() {
            break;
        }
        let ch = rest[0];
        if in_shift {
            if is_base64(ch) {
                buffer = (buffer << 6) | from_base64(ch);
                bits += 6;
                ctx.advance(1);
                if bits >= 16 {
                    let out_ch = buffer >> (bits - 16);
                    bits -= 16;
                    buffer &= (1 << bits) - 1;
                    if surrogate != 0 {
                        if (0xdc00..=0xdfff).contains(&out_ch) {
                            let code = (((surrogate & 0x3ff) << 10) | (out_ch & 0x3ff)) + 0x10000;
                            if let Some(cp) = CodePoint::from_u32(code) {
                                out.push(cp);
                            }
                            surrogate = 0;
                            continue;
                        }
                        if let Some(cp) = CodePoint::from_u32(surrogate) {
                            out.push(cp);
                        }
                        surrogate = 0;
                    }
                    if (0xd800..=0xdbff).contains(&out_ch) {
                        surrogate = out_ch;
                    } else if let Some(cp) = CodePoint::from_u32(out_ch) {
                        out.push(cp);
                    }
                }
            } else {
                in_shift = false;
                if bits >= 6 {
                    ctx.advance(1);
                    let start = startinpos;
                    let end = ctx.position();
                    let replace = ctx.handle_error(
                        errors,
                        start..end,
                        Some("partial character in shift sequence"),
                    )?;
                    out.push_wtf8(replace.as_ref());
                    continue;
                } else if bits > 0 && buffer != 0 {
                    ctx.advance(1);
                    let start = startinpos;
                    let end = ctx.position();
                    let replace = ctx.handle_error(
                        errors,
                        start..end,
                        Some("non-zero padding bits in shift sequence"),
                    )?;
                    out.push_wtf8(replace.as_ref());
                    continue;
                }
                if surrogate != 0
                    && decode_direct(ch)
                    && let Some(cp) = CodePoint::from_u32(surrogate)
                {
                    out.push(cp);
                }
                surrogate = 0;
                if ch == b'-' {
                    ctx.advance(1);
                }
            }
        } else if ch == b'+' {
            startinpos = ctx.position();
            if rest.len() >= 2 && rest[1] == b'-' {
                ctx.advance(2);
                out.push_char('+');
            } else if rest.len() >= 2 && !is_base64(rest[1]) {
                let start = ctx.position();
                let replace =
                    ctx.handle_error(errors, start..start + 2, Some("ill-formed sequence"))?;
                out.push_wtf8(replace.as_ref());
            } else {
                ctx.advance(1);
                in_shift = true;
                surrogate = 0;
                shift_out_start = out.len();
                bits = 0;
                buffer = 0;
            }
        } else if decode_direct(ch) {
            out.push_char(ch as char);
            ctx.advance(1);
        } else {
            let start = ctx.position();
            ctx.advance(1);
            let replace = ctx.handle_error(
                errors,
                start..ctx.position(),
                Some("unexpected special character"),
            )?;
            out.push_wtf8(replace.as_ref());
        }
    }
    let mut consumed = ctx.position();
    if in_shift && final_decode {
        if surrogate != 0 || bits >= 6 || (bits > 0 && buffer != 0) {
            let start = startinpos;
            let end = ctx.position();
            let replace =
                ctx.handle_error(errors, start..end, Some("unterminated shift sequence"))?;
            out.push_wtf8(replace.as_ref());
        }
    } else if in_shift {
        consumed = startinpos;
        out.truncate(shift_out_start);
    }
    Ok((out, consumed))
}
