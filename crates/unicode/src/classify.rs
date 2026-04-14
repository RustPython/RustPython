use icu_properties::props::{BidiClass, EnumeratedProperty, GeneralCategory};
use ucd::{Codepoint, NumericType};

use crate::{char_from_codepoint, is_surrogate};

pub fn general_category(cp: u32) -> GeneralCategory {
    if is_surrogate(cp) {
        GeneralCategory::Surrogate
    } else {
        char_from_codepoint(cp).map_or(GeneralCategory::Unassigned, GeneralCategory::for_char)
    }
}

pub fn is_alpha(cp: u32) -> bool {
    char_from_codepoint(cp).is_some_and(char::is_alphabetic)
}

pub fn is_alnum(cp: u32) -> bool {
    char_from_codepoint(cp).is_some_and(char::is_alphanumeric)
}

pub fn is_decimal(cp: u32) -> bool {
    matches!(general_category(cp), GeneralCategory::DecimalNumber)
}

pub fn is_digit(cp: u32) -> bool {
    char_from_codepoint(cp).is_some_and(|ch| {
        matches!(
            ch.numeric_type(),
            Some(NumericType::Decimal) | Some(NumericType::Digit)
        )
    })
}

pub fn is_numeric(cp: u32) -> bool {
    char_from_codepoint(cp).is_some_and(|ch| ch.numeric_value().is_some())
}

pub fn is_space(cp: u32) -> bool {
    char_from_codepoint(cp).is_some_and(|ch| {
        matches!(general_category(cp), GeneralCategory::SpaceSeparator)
            || matches!(
                BidiClass::for_char(ch),
                BidiClass::WhiteSpace | BidiClass::ParagraphSeparator | BidiClass::SegmentSeparator
            )
    })
}

/// Python's `str.isprintable()` semantics, which treat ASCII space as printable.
pub fn is_printable(cp: u32) -> bool {
    cp == '\u{0020}' as u32 || is_repr_printable(cp)
}

/// Repr/escape printable semantics, which exclude all Unicode space separators.
pub fn is_repr_printable(cp: u32) -> bool {
    !matches!(
        general_category(cp),
        GeneralCategory::SpaceSeparator
            | GeneralCategory::LineSeparator
            | GeneralCategory::ParagraphSeparator
            | GeneralCategory::Control
            | GeneralCategory::Format
            | GeneralCategory::Surrogate
            | GeneralCategory::PrivateUse
            | GeneralCategory::Unassigned
    )
}
