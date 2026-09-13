//! UTF-16 encode / incremental decode.

use super::wide::{self, emit_utf16, is_surrogate, push_codepoint, read_u16, resolve_bom};
use super::*;
use crate::wtf8::Wtf8Buf;

pub use super::wide::ByteOrder;

pub const ENCODING_NAME: &str = "utf-16";
pub const ENCODING_NAME_LE: &str = "utf-16-le";
pub const ENCODING_NAME_BE: &str = "utf-16-be";

const ERR_REASON: &str = "surrogates not allowed";

pub fn encode<Ctx, E>(
    ctx: Ctx,
    errors: &E,
    order: ByteOrder,
    bom: bool,
) -> Result<Vec<u8>, Ctx::Error>
where
    Ctx: EncodeContext,
    E: EncodeErrorHandler<Ctx>,
{
    wide::encode_wide(ctx, errors, order.is_big_endian(), bom, 2, ERR_REASON)
}

/// Decode one incremental chunk.
///
/// Returns `(text, consumed, byteorder)` where `byteorder` is CPython's
/// `-1` / `0` / `1`.
pub fn decode<Ctx, E>(
    mut ctx: Ctx,
    errors: &E,
    order: ByteOrder,
    final_decode: bool,
) -> Result<(Wtf8Buf, usize, i32), Ctx::Error>
where
    Ctx: DecodeContext,
    E: DecodeErrorHandler<Ctx>,
{
    let (big_endian, skip, byteorder) = resolve_bom(ctx.remaining_data(), order, 2);
    ctx.advance(skip);
    let mut out = Wtf8Buf::new();
    loop {
        let rest = ctx.remaining_data();
        if rest.len() < 2 {
            if rest.is_empty() || !final_decode {
                break;
            }
            let start = ctx.position();
            let end = ctx.full_data().len();
            let replace = ctx.handle_error(errors, start..end, Some("truncated data"))?;
            out.push_wtf8(replace.as_ref());
            continue;
        }
        let ch = read_u16(rest, big_endian);
        if !is_surrogate(ch as u32) {
            push_codepoint(&mut out, ch as u32);
            ctx.advance(2);
            continue;
        }
        if ch >= 0xdc00 {
            let start = ctx.position();
            let replace = ctx.handle_error(errors, start..start + 2, Some("illegal encoding"))?;
            out.push_wtf8(replace.as_ref());
            continue;
        }
        if rest.len() < 4 {
            if !final_decode {
                break;
            }
            let start = ctx.position();
            let end = ctx.full_data().len();
            let replace = ctx.handle_error(errors, start..end, Some("unexpected end of data"))?;
            out.push_wtf8(replace.as_ref());
            continue;
        }
        let ch2 = read_u16(&rest[2..], big_endian);
        if (0xdc00..=0xdfff).contains(&ch2) {
            let c = (((ch as u32 & 0x3ff) << 10) | (ch2 as u32 & 0x3ff)) + 0x10000;
            push_codepoint(&mut out, c);
            ctx.advance(4);
        } else {
            let start = ctx.position();
            let replace =
                ctx.handle_error(errors, start..start + 2, Some("illegal UTF-16 surrogate"))?;
            out.push_wtf8(replace.as_ref());
        }
    }
    Ok((out, ctx.position(), byteorder))
}

/// Encode a scalar (including a surrogate) as one or two UTF-16 units.
pub fn encode_codepoint(out: &mut Vec<u8>, cp: u32, order: ByteOrder) {
    emit_utf16(out, cp, order.is_big_endian());
}
