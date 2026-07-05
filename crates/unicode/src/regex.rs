//! Character-class and case predicates for the SRE regex engine.
//!
//! Every predicate takes a raw `u32` code point (SRE decodes strings into
//! `u32`s, including lone surrogates) and returns whether it belongs to the
//! class. ASCII-mode predicates only ever consider byte values; Unicode-mode
//! predicates consult the shared property tables.

use crate::classify;

const UNDERSCORE: u32 = '_' as u32;

const fn is_py_ascii_whitespace(b: u8) -> bool {
    matches!(b, b'\t' | b'\n' | b'\x0C' | b'\r' | b' ' | b'\x0B')
}

#[must_use]
pub fn is_word(ch: u32) -> bool {
    ch == UNDERSCORE || u8::try_from(ch).is_ok_and(|x| x.is_ascii_alphanumeric())
}

#[must_use]
pub fn is_space(ch: u32) -> bool {
    u8::try_from(ch).is_ok_and(is_py_ascii_whitespace)
}

#[must_use]
pub fn is_digit(ch: u32) -> bool {
    u8::try_from(ch).is_ok_and(|x| x.is_ascii_digit())
}

#[must_use]
pub fn is_loc_alnum(ch: u32) -> bool {
    // FIXME: Ignore the locales
    u8::try_from(ch).is_ok_and(|x| x.is_ascii_alphanumeric())
}

#[must_use]
pub fn is_loc_word(ch: u32) -> bool {
    ch == UNDERSCORE || is_loc_alnum(ch)
}

#[must_use]
pub const fn is_linebreak(ch: u32) -> bool {
    ch == '\n' as u32
}

#[must_use]
pub fn lower_ascii(ch: u32) -> u32 {
    u8::try_from(ch).map_or(ch, |x| x.to_ascii_lowercase() as u32)
}

#[must_use]
pub fn lower_locate(ch: u32) -> u32 {
    // FIXME: Ignore the locales
    lower_ascii(ch)
}

#[must_use]
pub fn upper_locate(ch: u32) -> u32 {
    // FIXME: Ignore the locales
    u8::try_from(ch).map_or(ch, |x| x.to_ascii_uppercase() as u32)
}

#[must_use]
pub fn is_uni_digit(ch: u32) -> bool {
    // SRE_UNI_IS_DIGIT matches Unicode decimal digits (Py_UNICODE_ISDECIMAL),
    // not just ASCII 0-9.
    char::try_from(ch).is_ok_and(classify::is_decimal)
}

#[must_use]
pub fn is_uni_space(ch: u32) -> bool {
    // TODO: check with cpython
    is_space(ch)
        || matches!(
            ch,
            0x0009
                | 0x000A
                | 0x000B
                | 0x000C
                | 0x000D
                | 0x001C
                | 0x001D
                | 0x001E
                | 0x001F
                | 0x0020
                | 0x0085
                | 0x00A0
                | 0x1680
                | 0x2000
                | 0x2001
                | 0x2002
                | 0x2003
                | 0x2004
                | 0x2005
                | 0x2006
                | 0x2007
                | 0x2008
                | 0x2009
                | 0x200A
                | 0x2028
                | 0x2029
                | 0x202F
                | 0x205F
                | 0x3000
        )
}

#[must_use]
pub const fn is_uni_linebreak(ch: u32) -> bool {
    matches!(
        ch,
        0x000A | 0x000B | 0x000C | 0x000D | 0x001C | 0x001D | 0x001E | 0x0085 | 0x2028 | 0x2029
    )
}

#[must_use]
pub fn is_uni_alnum(ch: u32) -> bool {
    // TODO: check with cpython
    char::try_from(ch).is_ok_and(classify::is_alnum)
}

#[must_use]
pub fn is_uni_word(ch: u32) -> bool {
    ch == UNDERSCORE || is_uni_alnum(ch)
}

#[must_use]
pub fn lower_unicode(ch: u32) -> u32 {
    // TODO: check with cpython
    char::try_from(ch).map_or(ch, |x| x.to_lowercase().next().unwrap() as u32)
}

#[must_use]
pub fn upper_unicode(ch: u32) -> u32 {
    // TODO: check with cpython
    char::try_from(ch).map_or(ch, |x| x.to_uppercase().next().unwrap() as u32)
}
