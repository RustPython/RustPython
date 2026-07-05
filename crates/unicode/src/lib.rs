//! Runtime-independent CPython-compatible Unicode semantics and data.
//!
//! Every entry point operates on plain `char`/`u32`/`CodePoint`/`&Wtf8` values
//! so it can be shared by any Python runtime; argument extraction and Python
//! exception mapping stay with the caller. There is no global mutable state and
//! results depend only on inputs.

#![no_std]

extern crate alloc;

pub mod case;
pub mod classify;
pub mod data;
pub mod identifier;
pub mod normalize;

pub use data::{Ucd, character_name, lookup_character, unicode_version};
pub use normalize::{NormalizeForm, is_normalized, normalize};

#[cfg(test)]
mod tests {
    use rustpython_wtf8::{CodePoint, Wtf8Buf};

    use crate::{NormalizeForm, Ucd, character_name, is_normalized, lookup_character, normalize};

    fn cp(ch: char) -> CodePoint {
        CodePoint::from(ch)
    }

    #[test]
    fn data_queries_match_unicodedata_behavior() {
        let ucd = Ucd::new(true);
        assert_eq!(ucd.category(cp('A')), "Lu");
        assert_eq!(ucd.category(CodePoint::from_u32(0xD800).unwrap()), "Cs");
        assert_eq!(lookup_character("SNOWMAN"), Some('☃'));
        assert_eq!(character_name('☃').as_deref(), Some("SNOWMAN"));
        assert_eq!(ucd.decimal(cp('५')), Some(5));
        assert_eq!(ucd.digit(cp('²')), Some(2));
        let third = ucd.numeric(cp('⅓')).unwrap();
        assert!((third - 1.0 / 3.0).abs() < 1e-6, "got {third}");
    }

    #[test]
    fn ucd_3_2_0_view_differs_from_modern() {
        let legacy = Ucd::new(false);
        assert_eq!(legacy.unidata_version(), "3.2.0");
    }

    #[test]
    fn numeric_type_chain_holds() {
        use crate::classify::{is_decimal, is_digit, is_numeric};

        // isdecimal ⊂ isdigit ⊂ isnumeric
        for c in ('\0'..='\u{2FFFF}').filter_map(|c| char::from_u32(c as u32)) {
            if is_decimal(c) {
                assert!(is_digit(c), "{c:?} decimal but not digit");
            }
            if is_digit(c) {
                assert!(is_numeric(c), "{c:?} digit but not numeric");
            }
        }
        assert!(crate::classify::is_decimal('5'));
        assert!(!crate::classify::is_decimal('²'));
        assert!(crate::classify::is_digit('²'));
        assert!(!crate::classify::is_digit('⅓'));
        assert!(crate::classify::is_numeric('⅓'));
    }

    #[test]
    fn identifier_predicates() {
        use crate::identifier::{is_continue, is_start};

        assert!(is_start('_'));
        assert!(is_start('가'));
        assert!(!is_start('1'));
        assert!(is_continue('1'));
    }

    #[test]
    fn casefold_full_mappings() {
        use crate::case::casefold_str;

        // ß case-folds to "ss"
        assert_eq!(casefold_str("ß"), "ss");
        assert_eq!(casefold_str("Σ"), "σ");
    }

    #[test]
    fn normalization_round_trips() {
        let composed = Wtf8Buf::from("é");
        let decomposed = normalize(NormalizeForm::Nfd, &composed);
        assert_eq!(normalize(NormalizeForm::Nfc, &decomposed), composed);
        assert!(is_normalized(NormalizeForm::Nfc, "é".as_bytes()));
        assert!(!is_normalized(NormalizeForm::Nfd, "é".as_bytes()));
    }
}
