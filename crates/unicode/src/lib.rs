#![cfg_attr(not(feature = "casefold"), no_std)]

extern crate alloc;

pub mod case;
pub mod classify;
pub mod data;
pub mod identifier;
pub mod normalize;
pub mod regex;

pub use normalize::NormalizeForm;
pub use unic_ucd_age::{UNICODE_VERSION, UnicodeVersion};

use core::char;

pub(crate) fn char_from_codepoint(cp: u32) -> Option<char> {
    char::from_u32(cp)
}

pub(crate) const fn is_surrogate(cp: u32) -> bool {
    matches!(cp, 0xD800..=0xDFFF)
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;
    use rustpython_wtf8::Wtf8Buf;

    use crate::{NormalizeForm, case, classify, data, identifier, normalize, regex};

    #[test]
    fn printable_and_repr_printable_follow_python_rules() {
        assert!(classify::is_printable(' ' as u32));
        assert!(!classify::is_repr_printable(' ' as u32));
        assert!(!classify::is_printable('\n' as u32));
    }

    #[test]
    fn identifier_and_regex_predicates_share_unicode_tables() {
        assert!(identifier::is_python_identifier_start('_' as u32));
        assert!(identifier::is_python_identifier("유니코드"));
        assert!(regex::is_unicode_word('가' as u32));
        assert!(regex::is_unicode_digit('५' as u32));
        assert!(regex::is_unicode_space('\u{3000}' as u32));
    }

    #[test]
    fn case_and_normalization_helpers_support_full_mappings() {
        let upper: Vec<_> = case::to_uppercase('ß' as u32).iter().collect();
        assert_eq!(upper, vec!['S' as u32, 'S' as u32]);

        let text = Wtf8Buf::from("e\u{301}");
        assert_eq!(
            normalize::normalize(NormalizeForm::Nfc, &text),
            Wtf8Buf::from("é")
        );
        assert!(normalize::is_normalized(
            NormalizeForm::Nfd,
            &normalize::normalize(NormalizeForm::Nfd, &Wtf8Buf::from("é"))
        ));
    }

    #[test]
    fn unicode_data_queries_match_existing_unicodedata_behavior() {
        assert_eq!(data::category('A' as u32), "Lu");
        assert_eq!(data::category(0xD800), "Cs");
        assert_eq!(data::lookup("SNOWMAN"), Some('☃' as u32));
        assert_eq!(data::name('☃' as u32).as_deref(), Some("SNOWMAN"));
        assert_eq!(data::decimal('५' as u32), Some(5));
        assert_eq!(data::digit('²' as u32), Some(2));
        assert_eq!(
            data::numeric('⅓' as u32),
            Some(data::NumericValue::Rational(1, 3))
        );
    }
}
