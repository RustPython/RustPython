// spell-checker:disable
//! Unstable functions from [`core::char`]

use core::slice;

pub const MAX_LEN_UTF8: usize = 4;
pub const MAX_LEN_UTF16: usize = 2;

// UTF-8 ranges and tags for encoding characters
const TAG_CONT: u8 = 0b1000_0000;
const TAG_TWO_B: u8 = 0b1100_0000;
const TAG_THREE_B: u8 = 0b1110_0000;
const TAG_FOUR_B: u8 = 0b1111_0000;
const MAX_ONE_B: u32 = 0x80;
const MAX_TWO_B: u32 = 0x800;
const MAX_THREE_B: u32 = 0x10000;

#[inline]
#[must_use]
pub const fn len_utf8(code: u32) -> usize {
    match code {
        ..MAX_ONE_B => 1,
        ..MAX_TWO_B => 2,
        ..MAX_THREE_B => 3,
        _ => 4,
    }
}

#[inline]
#[must_use]
const fn len_utf16(code: u32) -> usize {
    if (code & 0xFFFF) == code { 1 } else { 2 }
}

/// Encodes a raw `u32` value as UTF-8 into the provided byte buffer,
/// and then returns the subslice of the buffer that contains the encoded character.
///
/// Unlike `char::encode_utf8`, this method also handles codepoints in the surrogate range.
/// (Creating a `char` in the surrogate range is UB.)
/// The result is valid [generalized UTF-8] but not valid UTF-8.
///
/// [generalized UTF-8]: https://simonsapin.github.io/wtf-8/#generalized-utf8
///
/// # Panics
///
/// Panics if the buffer is not large enough.
/// A buffer of length four is large enough to encode any `char`.
#[doc(hidden)]
#[inline]
pub fn encode_utf8_raw(code: u32, dst: &mut [u8]) -> &mut [u8] {
    let len = len_utf8(code);
    match (len, &mut *dst) {
        (1, [a, ..]) => {
            *a = code as u8;
        }
        (2, [a, b, ..]) => {
            *a = (code >> 6 & 0x1F) as u8 | TAG_TWO_B;
            *b = (code & 0x3F) as u8 | TAG_CONT;
        }
        (3, [a, b, c, ..]) => {
            *a = (code >> 12 & 0x0F) as u8 | TAG_THREE_B;
            *b = (code >> 6 & 0x3F) as u8 | TAG_CONT;
            *c = (code & 0x3F) as u8 | TAG_CONT;
        }
        (4, [a, b, c, d, ..]) => {
            *a = (code >> 18 & 0x07) as u8 | TAG_FOUR_B;
            *b = (code >> 12 & 0x3F) as u8 | TAG_CONT;
            *c = (code >> 6 & 0x3F) as u8 | TAG_CONT;
            *d = (code & 0x3F) as u8 | TAG_CONT;
        }
        _ => {
            panic!(
                "encode_utf8: need {len} bytes to encode U+{code:04X} but buffer has just {dst_len}",
                dst_len = dst.len(),
            )
        }
    };
    // SAFETY: `<&mut [u8]>::as_mut_ptr` is guaranteed to return a valid pointer and `len` has been tested to be within bounds.
    unsafe { slice::from_raw_parts_mut(dst.as_mut_ptr(), len) }
}

/// Encodes a raw `u32` value as UTF-16 into the provided `u16` buffer,
/// and then returns the subslice of the buffer that contains the encoded character.
///
/// Unlike `char::encode_utf16`, this method also handles codepoints in the surrogate range.
/// (Creating a `char` in the surrogate range is UB.)
///
/// # Panics
///
/// Panics if the buffer is not large enough.
/// A buffer of length 2 is large enough to encode any `char`.
#[doc(hidden)]
#[inline]
pub fn encode_utf16_raw(mut code: u32, dst: &mut [u16]) -> &mut [u16] {
    let len = len_utf16(code);
    match (len, &mut *dst) {
        (1, [a, ..]) => {
            *a = code as u16;
        }
        (2, [a, b, ..]) => {
            code -= 0x1_0000;
            *a = (code >> 10) as u16 | 0xD800;
            *b = (code & 0x3FF) as u16 | 0xDC00;
        }
        _ => {
            panic!(
                "encode_utf16: need {len} bytes to encode U+{code:04X} but buffer has just {dst_len}",
                dst_len = dst.len(),
            )
        }
    };
    // SAFETY: `<&mut [u16]>::as_mut_ptr` is guaranteed to return a valid pointer and `len` has been tested to be within bounds.
    unsafe { slice::from_raw_parts_mut(dst.as_mut_ptr(), len) }
}
