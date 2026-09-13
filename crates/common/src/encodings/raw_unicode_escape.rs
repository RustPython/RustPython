//! `raw-unicode-escape` encode / incremental decode.

use super::*;
use crate::wtf8::{CodePoint, Wtf8, Wtf8Buf};

pub const ENCODING_NAME: &str = "rawunicodeescape";

/// Encode `s`. Code points below 0x100 stay as a Latin-1 byte; the rest
/// become `\uXXXX` / `\UXXXXXXXX`.
#[must_use]
pub fn encode_bytes(s: &Wtf8) -> Vec<u8> {
    let mut out = Vec::new();
    for cp in s.code_points() {
        let v = cp.to_u32();
        if v < 0x100 {
            out.push(v as u8);
        } else if v < 0x10000 {
            out.extend_from_slice(format!("\\u{v:04x}").as_bytes());
        } else {
            out.extend_from_slice(format!("\\U{v:08x}").as_bytes());
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
    loop {
        let rest = ctx.remaining_data();
        if rest.is_empty() {
            break;
        }
        let b = rest[0];
        if b != b'\\' {
            out.push_char(b as char);
            ctx.advance(1);
            continue;
        }
        let kind = rest.get(1).copied();
        let want = match kind {
            Some(b'u') => 4usize,
            Some(b'U') => 8usize,
            _ => 0,
        };
        if want != 0 {
            let escape_start = ctx.position();
            let digits_start = 2;
            if !final_decode && rest.len() < digits_start + want {
                break;
            }
            let available = (digits_start + want).min(rest.len());
            let mut hex_end = digits_start;
            while hex_end < available && rest[hex_end].is_ascii_hexdigit() {
                hex_end += 1;
            }
            let numeric = if available == digits_start + want && hex_end == available {
                core::str::from_utf8(&rest[digits_start..available])
                    .ok()
                    .and_then(|s| u32::from_str_radix(s, 16).ok())
            } else {
                None
            };
            if let Some(c) = numeric.and_then(CodePoint::from_u32) {
                out.push(c);
                ctx.advance(available);
                continue;
            }
            let error_end = ctx.position()
                + if numeric.is_some() {
                    available
                } else {
                    hex_end
                };
            let reason = if numeric.is_some() {
                "illegal Unicode character"
            } else if want == 4 {
                "truncated \\uXXXX escape"
            } else {
                "truncated \\UXXXXXXXX escape"
            };
            let replace = ctx.handle_error(errors, escape_start..error_end, Some(reason))?;
            out.push_wtf8(replace.as_ref());
            continue;
        }
        if kind.is_none() && !final_decode {
            break;
        }
        out.push_char(b as char);
        ctx.advance(1);
        if let Some(next) = kind {
            out.push_char(next as char);
            ctx.advance(1);
        }
    }
    Ok((out, ctx.position()))
}
