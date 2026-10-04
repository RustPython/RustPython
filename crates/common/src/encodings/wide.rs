//! Shared utf-16 / utf-32 unit helpers.

use super::*;
use crate::wtf8::CodePoint;

/// Byte order for a wide Unicode encoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ByteOrder {
    /// Platform endianness; a leading BOM selects the other side on decode.
    Native,
    Little,
    Big,
}

impl ByteOrder {
    #[must_use]
    pub const fn is_big_endian(self) -> bool {
        match self {
            Self::Native => cfg!(target_endian = "big"),
            Self::Little => false,
            Self::Big => true,
        }
    }
}

pub(super) fn push_u16(out: &mut Vec<u8>, unit: u16, big_endian: bool) {
    out.extend_from_slice(&if big_endian {
        unit.to_be_bytes()
    } else {
        unit.to_le_bytes()
    });
}

pub(super) fn push_u32(out: &mut Vec<u8>, unit: u32, big_endian: bool) {
    out.extend_from_slice(&if big_endian {
        unit.to_be_bytes()
    } else {
        unit.to_le_bytes()
    });
}

pub(super) fn emit_utf16(out: &mut Vec<u8>, cp: u32, big_endian: bool) {
    if cp <= 0xffff {
        push_u16(out, cp as u16, big_endian);
    } else {
        let v = cp - 0x10000;
        push_u16(out, 0xd800 | ((v >> 10) as u16), big_endian);
        push_u16(out, 0xdc00 | ((v & 0x3ff) as u16), big_endian);
    }
}

pub(super) fn emit_utf32(out: &mut Vec<u8>, cp: u32, big_endian: bool) {
    push_u32(out, cp, big_endian);
}

pub(super) fn read_u16(data: &[u8], big_endian: bool) -> u16 {
    let arr = [data[0], data[1]];
    if big_endian {
        u16::from_be_bytes(arr)
    } else {
        u16::from_le_bytes(arr)
    }
}

pub(super) fn read_u32(data: &[u8], big_endian: bool) -> u32 {
    let arr = [data[0], data[1], data[2], data[3]];
    if big_endian {
        u32::from_be_bytes(arr)
    } else {
        u32::from_le_bytes(arr)
    }
}

pub(super) const fn is_surrogate(cp: u32) -> bool {
    matches!(cp, 0xd800..=0xdfff)
}

/// `(big_endian, bom_len, reported_byteorder)` for a native or fixed decode.
///
/// `reported_byteorder` is CPython's `-1` / `0` / `1` (`le` / native-no-BOM / `be`).
pub(super) fn resolve_bom(data: &[u8], order: ByteOrder, unit: usize) -> (bool, usize, i32) {
    match order {
        ByteOrder::Little => (false, 0, -1),
        ByteOrder::Big => (true, 0, 1),
        ByteOrder::Native if unit == 2 && data.starts_with(&[0xff, 0xfe]) => (false, 2, -1),
        ByteOrder::Native if unit == 2 && data.starts_with(&[0xfe, 0xff]) => (true, 2, 1),
        ByteOrder::Native if unit == 4 && data.starts_with(&[0xff, 0xfe, 0x00, 0x00]) => {
            (false, 4, -1)
        }
        ByteOrder::Native if unit == 4 && data.starts_with(&[0x00, 0x00, 0xfe, 0xff]) => {
            (true, 4, 1)
        }
        ByteOrder::Native => (cfg!(target_endian = "big"), 0, 0),
    }
}

pub(super) fn encode_wide<Ctx, E>(
    mut ctx: Ctx,
    errors: &E,
    big_endian: bool,
    bom: bool,
    unit: usize,
    reason: &str,
) -> Result<Vec<u8>, Ctx::Error>
where
    Ctx: EncodeContext,
    E: EncodeErrorHandler<Ctx>,
{
    let emit = if unit == 2 { emit_utf16 } else { emit_utf32 };
    let mut out = Vec::new();
    if bom {
        emit(&mut out, 0xfeff, big_endian);
    }
    loop {
        let data = ctx.remaining_data();
        let mut iter = iter_code_points(data);
        let Some((i, _)) = iter.find(|(_, c)| is_surrogate(c.to_u32())) else {
            for (_, c) in iter_code_points(data) {
                emit(&mut out, c.to_u32(), big_endian);
            }
            break;
        };
        for (_, c) in iter_code_points(&data[..i.bytes]) {
            emit(&mut out, c.to_u32(), big_endian);
        }
        let err_end = match { iter }.find(|(_, c)| !is_surrogate(c.to_u32())) {
            Some((j, _)) => ctx.position() + j,
            None => ctx.data_len(),
        };
        let range = (ctx.position() + i)..err_end;
        let replace = ctx.handle_error(errors, range.clone(), Some(reason))?;
        match replace {
            EncodeReplace::Str(s) => {
                for c in s.as_ref().code_points() {
                    let cp = c.to_u32();
                    if is_surrogate(cp) {
                        return Err(ctx.error_encoding(range, Some(reason)));
                    }
                    emit(&mut out, cp, big_endian);
                }
            }
            EncodeReplace::Bytes(b) => {
                if b.as_ref().len() % unit != 0 {
                    return Err(ctx.error_encoding(range, Some(reason)));
                }
                out.extend_from_slice(b.as_ref());
            }
        }
    }
    Ok(out)
}

pub(super) fn push_codepoint(out: &mut Wtf8Buf, cp: u32) {
    if let Some(c) = CodePoint::from_u32(cp) {
        out.push(c);
    }
}
