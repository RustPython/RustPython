use alloc::{format, string::String, vec::Vec};

use icu_properties::{
    CodePointSetData,
    props::{
        BidiClass, BidiMirrored, CanonicalCombiningClass, EastAsianWidth, EnumeratedProperty,
        NamedEnumeratedProperty,
    },
};
use itertools::Itertools;
use ucd::{Codepoint, DecompositionType, Number, NumericType};
use unic_ucd_age::{Age, UnicodeVersion};

use crate::{char_from_codepoint, classify, is_surrogate};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NumericValue {
    Integer(i64),
    Rational(i64, i64),
}

pub fn is_assigned_in_version(cp: u32, version: UnicodeVersion) -> bool {
    if is_surrogate(cp) {
        true
    } else {
        char_from_codepoint(cp)
            .is_some_and(|ch| Age::of(ch).is_some_and(|age| age.actual() <= version))
    }
}

pub fn category(cp: u32) -> &'static str {
    classify::general_category(cp).short_name()
}

pub fn lookup(name: &str) -> Option<u32> {
    unicode_names2::character(name).map(u32::from)
}

pub fn name(cp: u32) -> Option<String> {
    char_from_codepoint(cp)
        .and_then(unicode_names2::name)
        .map(|name| name.collect())
}

pub fn bidirectional(cp: u32) -> &'static str {
    char_from_codepoint(cp)
        .map_or(BidiClass::LeftToRight, BidiClass::for_char)
        .short_name()
}

pub fn east_asian_width(cp: u32) -> &'static str {
    char_from_codepoint(cp)
        .map_or(EastAsianWidth::Neutral, EastAsianWidth::for_char)
        .short_name()
}

pub fn mirrored(cp: u32) -> bool {
    char_from_codepoint(cp).is_some_and(|ch| CodePointSetData::new::<BidiMirrored>().contains(ch))
}

pub fn combining(cp: u32) -> u8 {
    char_from_codepoint(cp).map_or(0, |ch| {
        CanonicalCombiningClass::for_char(ch).to_icu4c_value()
    })
}

pub fn decomposition(cp: u32) -> String {
    let ch = match char_from_codepoint(cp) {
        Some(ch) => ch,
        None => return String::new(),
    };
    let chars: Vec<char> = ch.decomposition_map().collect();
    if chars.len() == 1 && chars[0] == ch {
        return String::new();
    }
    let hex_parts = chars.iter().map(|c| format!("{:04X}", *c as u32)).join(" ");
    match ch.decomposition_type() {
        Some(DecompositionType::Canonical) | None => hex_parts,
        Some(dt) => format!("<{}> {hex_parts}", decomposition_type_tag(dt)),
    }
}

pub fn digit(cp: u32) -> Option<u32> {
    let ch = char_from_codepoint(cp)?;
    if matches!(
        ch.numeric_type(),
        Some(NumericType::Decimal) | Some(NumericType::Digit)
    ) && let Some(Number::Integer(value)) = ch.numeric_value()
    {
        return u32::try_from(value).ok();
    }
    None
}

pub fn decimal(cp: u32) -> Option<u32> {
    let ch = char_from_codepoint(cp)?;
    if ch.numeric_type() == Some(NumericType::Decimal)
        && let Some(Number::Integer(value)) = ch.numeric_value()
    {
        return u32::try_from(value).ok();
    }
    None
}

pub fn numeric(cp: u32) -> Option<NumericValue> {
    match char_from_codepoint(cp)?.numeric_value()? {
        Number::Integer(value) => Some(NumericValue::Integer(value)),
        Number::Rational(num, den) => Some(NumericValue::Rational(num.into(), den.into())),
    }
}

fn decomposition_type_tag(dt: DecompositionType) -> &'static str {
    match dt {
        DecompositionType::Canonical => "canonical",
        DecompositionType::Compat => "compat",
        DecompositionType::Circle => "circle",
        DecompositionType::Final => "final",
        DecompositionType::Font => "font",
        DecompositionType::Fraction => "fraction",
        DecompositionType::Initial => "initial",
        DecompositionType::Isolated => "isolated",
        DecompositionType::Medial => "medial",
        DecompositionType::Narrow => "narrow",
        DecompositionType::Nobreak => "noBreak",
        DecompositionType::Small => "small",
        DecompositionType::Square => "square",
        DecompositionType::Sub => "sub",
        DecompositionType::Super => "super",
        DecompositionType::Vertical => "vertical",
        DecompositionType::Wide => "wide",
    }
}
