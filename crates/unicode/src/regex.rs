use crate::{case, classify};

const UNDERSCORE: u32 = '_' as u32;

const fn is_py_ascii_whitespace(byte: u8) -> bool {
    matches!(byte, b'\t' | b'\n' | b'\x0C' | b'\r' | b' ' | b'\x0B')
}

pub fn is_word(cp: u32) -> bool {
    cp == UNDERSCORE
        || u8::try_from(cp)
            .map(|byte| byte.is_ascii_alphanumeric())
            .unwrap_or(false)
}

pub fn is_space(cp: u32) -> bool {
    u8::try_from(cp)
        .map(is_py_ascii_whitespace)
        .unwrap_or(false)
}

pub fn is_digit(cp: u32) -> bool {
    u8::try_from(cp)
        .map(|byte| byte.is_ascii_digit())
        .unwrap_or(false)
}

pub fn is_locale_alnum(cp: u32) -> bool {
    u8::try_from(cp)
        .map(|byte| byte.is_ascii_alphanumeric())
        .unwrap_or(false)
}

pub fn is_locale_word(cp: u32) -> bool {
    cp == UNDERSCORE || is_locale_alnum(cp)
}

pub const fn is_linebreak(cp: u32) -> bool {
    cp == '\n' as u32
}

pub fn lower_ascii(cp: u32) -> u32 {
    u8::try_from(cp)
        .map(|byte| byte.to_ascii_lowercase() as u32)
        .unwrap_or(cp)
}

pub fn lower_locale(cp: u32) -> u32 {
    lower_ascii(cp)
}

pub fn upper_locale(cp: u32) -> u32 {
    u8::try_from(cp)
        .map(|byte| byte.to_ascii_uppercase() as u32)
        .unwrap_or(cp)
}

pub fn is_unicode_digit(cp: u32) -> bool {
    classify::is_decimal(cp)
}

pub fn is_unicode_space(cp: u32) -> bool {
    classify::is_space(cp)
}

pub const fn is_unicode_linebreak(cp: u32) -> bool {
    matches!(
        cp,
        0x000A | 0x000B | 0x000C | 0x000D | 0x001C | 0x001D | 0x001E | 0x0085 | 0x2028 | 0x2029
    )
}

pub fn is_unicode_alnum(cp: u32) -> bool {
    classify::is_alnum(cp)
}

pub fn is_unicode_word(cp: u32) -> bool {
    cp == UNDERSCORE || is_unicode_alnum(cp)
}

pub fn lower_unicode(cp: u32) -> u32 {
    case::to_lowercase(cp).first().unwrap_or(cp)
}

pub fn upper_unicode(cp: u32) -> u32 {
    case::to_uppercase(cp).first().unwrap_or(cp)
}
