//! UTF-32 encode / incremental decode.

use super::wide::{self, emit_utf32, is_surrogate, push_codepoint, read_u32, resolve_bom};
use super::*;
use crate::wtf8::Wtf8Buf;

pub use super::wide::ByteOrder;

pub const ENCODING_NAME: &str = "utf-32";
pub const ENCODING_NAME_LE: &str = "utf-32-le";
pub const ENCODING_NAME_BE: &str = "utf-32-be";

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
    wide::encode_wide(ctx, errors, order.is_big_endian(), bom, 4, ERR_REASON)
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
    let (big_endian, skip, byteorder) = resolve_bom(ctx.remaining_data(), order, 4);
    ctx.advance(skip);
    let mut out = Wtf8Buf::new();
    loop {
        let rest = ctx.remaining_data();
        if rest.len() < 4 {
            if rest.is_empty() || !final_decode {
                break;
            }
            let start = ctx.position();
            let end = ctx.full_data().len();
            let replace = ctx.handle_error(errors, start..end, Some("truncated data"))?;
            out.push_wtf8(replace.as_ref());
            continue;
        }
        let ch = read_u32(rest, big_endian);
        if is_surrogate(ch) {
            let start = ctx.position();
            let replace = ctx.handle_error(
                errors,
                start..start + 4,
                Some("code point in surrogate code point range(0xd800, 0xe000)"),
            )?;
            out.push_wtf8(replace.as_ref());
            continue;
        }
        if ch >= 0x110000 {
            let start = ctx.position();
            let replace = ctx.handle_error(
                errors,
                start..start + 4,
                Some("code point not in range(0x110000)"),
            )?;
            out.push_wtf8(replace.as_ref());
            continue;
        }
        push_codepoint(&mut out, ch);
        ctx.advance(4);
    }
    Ok((out, ctx.position(), byteorder))
}

pub fn encode_codepoint(out: &mut Vec<u8>, cp: u32, order: ByteOrder) {
    emit_utf32(out, cp, order.is_big_endian());
}
