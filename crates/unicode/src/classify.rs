//! Character classification predicates for Python `str` methods.
//!
//! Each predicate operates on a single Unicode scalar. Callers iterating over
//! WTF-8 text apply these per code point, treating lone surrogates as failing
//! every predicate.

use icu_properties::props::{
    BidiClass, EnumeratedProperty, GeneralCategory, GeneralCategoryGroup, NumericType,
};

/// `str.isalpha` for a single character: any `Letter` general category.
#[must_use]
pub fn is_alpha(c: char) -> bool {
    GeneralCategoryGroup::Letter.contains(GeneralCategory::for_char(c))
}

/// `str.isalnum` for a single character: any `Letter` or `Number` category.
#[must_use]
pub fn is_alnum(c: char) -> bool {
    GeneralCategoryGroup::Letter
        .union(GeneralCategoryGroup::Number)
        .contains(GeneralCategory::for_char(c))
}

/// `str.isdecimal` for a single character: `Decimal_Number` general category.
#[must_use]
pub fn is_decimal(c: char) -> bool {
    matches!(GeneralCategory::for_char(c), GeneralCategory::DecimalNumber)
}

/// `str.isdigit` for a single character: `Numeric_Type` of `Digit` or `Decimal`.
#[must_use]
pub fn is_digit(c: char) -> bool {
    matches!(
        NumericType::for_char(c),
        NumericType::Digit | NumericType::Decimal
    )
}

/// `str.isnumeric` for a single character: any numeric `Numeric_Type`.
#[must_use]
pub fn is_numeric(c: char) -> bool {
    matches!(
        NumericType::for_char(c),
        NumericType::Decimal | NumericType::Digit | NumericType::Numeric
    )
}

/// `str.isspace` for a single character: `Space_Separator`, or a bidi
/// whitespace / paragraph / segment separator.
#[must_use]
pub fn is_space(c: char) -> bool {
    matches!(
        GeneralCategory::for_char(c),
        GeneralCategory::SpaceSeparator
    ) || matches!(
        BidiClass::for_char(c),
        BidiClass::WhiteSpace | BidiClass::ParagraphSeparator | BidiClass::SegmentSeparator
    )
}

/// `str.isprintable` for a single character: ASCII space is printable, as are
/// all characters that survive [`is_repr_printable`].
#[must_use]
pub fn is_printable(c: char) -> bool {
    c == '\u{0020}' || is_repr_printable(c)
}

/// Repr/escape printable semantics.
///
/// The following categories are not printable:
/// * Cc (Other, Control)
/// * Cf (Other, Format)
/// * Cs (Other, Surrogate)
/// * Co (Other, Private Use)
/// * Cn (Other, Not Assigned)
/// * Zl (Separator, Line)
/// * Zp (Separator, Paragraph)
/// * Zs (Separator, Space), including ASCII space
#[must_use]
pub fn is_repr_printable(c: char) -> bool {
    !matches!(
        GeneralCategory::for_char(c),
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

#[cfg(test)]
mod tests {
    use super::{is_decimal, is_digit, is_numeric};

    #[test]
    fn numeric_type_chain_holds() {
        // isdecimal ⊂ isdigit ⊂ isnumeric
        for c in ('\0'..='\u{2FFFF}').filter_map(|c| char::from_u32(c as u32)) {
            if is_decimal(c) {
                assert!(is_digit(c), "{c:?} decimal but not digit");
            }
            if is_digit(c) {
                assert!(is_numeric(c), "{c:?} digit but not numeric");
            }
        }
        assert!(is_decimal('5'));
        assert!(!is_decimal('²'));
        assert!(is_digit('²'));
        assert!(!is_digit('⅓'));
        assert!(is_numeric('⅓'));
    }
}
