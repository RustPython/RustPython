//! Case mapping, case folding, and casing predicates for Python string casing.
//!
//! Code-point mappings (`simple_*`) return a single `char` and back the SRE
//! engine's `IGNORECASE` handling. String-level helpers (`capitalize`, `title`,
//! `swapcase`, `casefold`) implement the full, context-sensitive mappings used
//! by `str` methods and pass lone surrogates through unchanged. The casing
//! predicates expose the derived properties that `str.islower`/`isupper`/
//! `istitle` need.
//!
//! All Unicode mappings use ICU data, matching the rest of this crate rather
//! than the Unicode version bundled with the Rust compiler.

// spell-checker:ignore ΟΔΟΣ Οδος

use alloc::{
    string::{String, ToString},
    vec::Vec,
};

use icu_casemap::options::{LeadingAdjustment, TitlecaseOptions};
use icu_casemap::{CaseMapper, TitlecaseMapper};
use icu_locale::LanguageIdentifier;
use icu_properties::props::{EnumeratedProperty, GeneralCategory};
use rustpython_wtf8::{CodePoint, Wtf8, Wtf8Buf, Wtf8Chunk};
use writeable::Writeable;

include!(concat!(env!("OUT_DIR"), "/generated/case_mappings.rs"));

#[inline(always)]
fn lookup_page(ch: char, index: &[u16], pages: &[u16]) -> usize {
    let n = ch as usize;
    usize::from(pages[usize::from(index[n >> 8]) * 256 + (n & 255)])
}

// Code-point mappings

/// Simple (one-to-one) lowercase mapping of `c` (`Py_UNICODE_TOLOWER`).
#[must_use]
pub fn simple_lowercase(c: char) -> char {
    if c.is_ascii() {
        return c.to_ascii_lowercase();
    }
    CaseMapper::new().simple_lowercase(c)
}

/// Simple (one-to-one) uppercase mapping of `c` (`Py_UNICODE_TOUPPER`).
#[must_use]
pub fn simple_uppercase(c: char) -> char {
    if c.is_ascii() {
        return c.to_ascii_uppercase();
    }
    CaseMapper::new().simple_uppercase(c)
}

/// Simple (one-to-one) titlecase mapping of `c` (`Py_UNICODE_TOTITLE`).
#[must_use]
pub fn simple_titlecase(c: char) -> char {
    CaseMapper::new().simple_titlecase(c)
}

/// Simple (one-to-one) case fold of `c`.
#[must_use]
pub fn simple_fold(c: char) -> char {
    CaseMapper::new().simple_fold(c)
}

// Casing predicates

/// Whether `c` has the `Lowercase` property.
#[must_use]
pub fn is_lowercase(c: char) -> bool {
    lookup_page(c, CASE_FLAGS_INDEX, CASE_FLAGS_PAGES) & 4 != 0
}

/// Whether `c` has the `Uppercase` property.
#[must_use]
pub fn is_uppercase(c: char) -> bool {
    lookup_page(c, CASE_FLAGS_INDEX, CASE_FLAGS_PAGES) & 8 != 0
}

/// Whether `c` is a titlecase letter (general category `Lt`).
#[must_use]
pub fn is_titlecase(c: char) -> bool {
    GeneralCategory::for_char(c) == GeneralCategory::TitlecaseLetter
}

/// Whether `c` has the `Cased` property.
#[must_use]
pub fn is_cased(c: char) -> bool {
    lookup_page(c, CASE_FLAGS_INDEX, CASE_FLAGS_PAGES) & 1 != 0
}

/// Whether `c` has the `Case_Ignorable` property.
#[must_use]
pub fn is_case_ignorable(c: char) -> bool {
    lookup_page(c, CASE_FLAGS_INDEX, CASE_FLAGS_PAGES) & 2 != 0
}

// String-level mappings

#[inline]
fn ascii_prefix<const UPPERCASE: bool>(text: &str) -> (usize, String) {
    let end = text
        .bytes()
        .position(|b| !b.is_ascii())
        .unwrap_or(text.len());
    let mut out = Vec::with_capacity(text.len());
    if UPPERCASE {
        out.extend(text.as_bytes()[..end].iter().map(u8::to_ascii_uppercase));
    } else {
        out.extend(text.as_bytes()[..end].iter().map(u8::to_ascii_lowercase));
    }
    // SAFETY: the converted prefix contains only ASCII.
    (end, unsafe { String::from_utf8_unchecked(out) })
}

/// Lowercase `text` using the root locale and final-sigma context.
#[must_use]
pub fn lowercase_str(text: &str) -> String {
    let (prefix, mut out) = ascii_prefix::<false>(text);
    let mut chars = text[prefix..].chars();
    while let Some(ch) = chars.next() {
        if ch.is_ascii() {
            out.push(ch.to_ascii_lowercase());
        } else if ch == 'Σ' {
            out.push(handle_capital_sigma(
                text,
                text.len() - chars.as_str().len() - ch.len_utf8(),
            ));
        } else {
            let index = lookup_page(ch, LOWER_INDEX, LOWER_PAGES);
            push_mapping(ch, index, LOWER_MAPPINGS, |ch| out.push(ch));
        }
    }
    out
}

fn lowercase_utf8(text: &str, out: &mut FmtWriter<'_>) {
    for (i, ch) in text.char_indices() {
        lowercase_or_sigma(ch, text, i, out);
    }
}

/// Lowercase `text`, passing lone surrogates through unchanged.
#[must_use]
pub fn lowercase_wtf8(text: &Wtf8) -> Wtf8Buf {
    map_wtf8(text, lowercase_utf8)
}

/// Uppercase `text` using the root locale.
#[must_use]
pub fn uppercase_str(text: &str) -> String {
    let (prefix, mut out) = ascii_prefix::<true>(text);
    for ch in text[prefix..].chars() {
        if ch.is_ascii() {
            out.push(ch.to_ascii_uppercase());
        } else {
            let index = lookup_page(ch, UPPER_INDEX, UPPER_PAGES);
            push_mapping(ch, index, UPPER_MAPPINGS, |ch| out.push(ch));
        }
    }
    out
}

fn uppercase_utf8(text: &str, out: &mut FmtWriter<'_>) {
    for ch in text.chars() {
        uppercase_char(ch, out);
    }
}

/// Uppercase `text`, passing lone surrogates through unchanged.
#[must_use]
pub fn uppercase_wtf8(text: &Wtf8) -> Wtf8Buf {
    map_wtf8(text, uppercase_utf8)
}

/// Full Unicode case fold of `text` (`str.casefold`).
#[must_use]
pub fn casefold_str(text: &str) -> String {
    CaseMapper::new().fold_string(text).to_string()
}

/// Full Unicode case fold of `text`, passing lone surrogates through unchanged.
#[must_use]
pub fn casefold_wtf8(text: &Wtf8) -> Wtf8Buf {
    map_wtf8(text, |s, out| {
        CaseMapper::new()
            .fold(s)
            .write_to(out)
            .expect("writing to an in-memory buffer cannot fail");
    })
}

/// Capitalize `text` (`str.capitalize`): titlecase the first cased character,
/// lowercase the rest, with final-sigma context.
#[must_use]
pub fn capitalize_str(text: &str) -> String {
    let mut out = Vec::with_capacity(text.len());
    capitalize_utf8(text, &mut FmtWriter(&mut out));
    // SAFETY: capitalize_utf8 only appends valid UTF-8.
    unsafe { String::from_utf8_unchecked(out) }
}

/// Capitalize `text`, passing lone surrogates through unchanged.
///
/// Only the first character of the whole string is titlecased; every later
/// character (including the first of a run that follows a lone surrogate) is
/// lowercased.
#[must_use]
pub fn capitalize_wtf8(text: &Wtf8) -> Wtf8Buf {
    let mut out = Vec::with_capacity(text.len());
    let mut first = true;
    for chunk in text.chunks() {
        match chunk {
            Wtf8Chunk::Utf8(s) => {
                let mut writer = FmtWriter(&mut out);
                if first {
                    capitalize_utf8(s, &mut writer);
                    first = false;
                } else {
                    for (i, ch) in s.char_indices() {
                        lowercase_or_sigma(ch, s, i, &mut writer);
                    }
                }
            }
            Wtf8Chunk::Surrogate(c) => {
                first = false;
                push_surrogate(&mut out, c);
            }
        }
    }
    // SAFETY:
    // * capitalize_utf8 / lowercase_or_sigma only append valid UTF-8.
    // * Surrogates are appended as valid WTF-8 (encoded via Wtf8Buf::push).
    unsafe { Wtf8Buf::from_bytes_unchecked(out) }
}

/// Title case `text` (`str.title`).
#[must_use]
pub fn title_str(text: &str) -> String {
    let mut out = Vec::with_capacity(text.len());
    titlecase_string(text, &mut FmtWriter(&mut out));
    // SAFETY: titlecase_string only appends valid UTF-8.
    unsafe { String::from_utf8_unchecked(out) }
}

/// Title case `text`, passing lone surrogates through unchanged.
#[must_use]
pub fn title_wtf8(text: &Wtf8) -> Wtf8Buf {
    map_wtf8(text, titlecase_string)
}

/// Swap the case of every character in `text` (`str.swapcase`).
#[must_use]
pub fn swapcase_str(text: &str) -> String {
    let mut out = Vec::with_capacity(text.len());
    swapcase_utf8(text, &mut FmtWriter(&mut out));
    // SAFETY: swapcase_utf8 only appends valid UTF-8.
    unsafe { String::from_utf8_unchecked(out) }
}

/// Swap the case of every character in `text`, passing lone surrogates through.
#[must_use]
pub fn swapcase_wtf8(text: &Wtf8) -> Wtf8Buf {
    map_wtf8(text, swapcase_utf8)
}

// Internal helpers

/// Run `f` over each valid UTF-8 run of `text`, appending the mapped output and
/// carrying lone surrogates through unchanged.
fn map_wtf8(text: &Wtf8, f: impl Fn(&str, &mut FmtWriter<'_>)) -> Wtf8Buf {
    let mut out = Vec::with_capacity(text.len());
    for chunk in text.chunks() {
        match chunk {
            Wtf8Chunk::Utf8(s) => f(s, &mut FmtWriter(&mut out)),
            Wtf8Chunk::Surrogate(c) => push_surrogate(&mut out, c),
        }
    }
    // SAFETY:
    // * `f` only appends valid UTF-8.
    // * Surrogates are appended as valid WTF-8 (encoded via Wtf8Buf::push).
    unsafe { Wtf8Buf::from_bytes_unchecked(out) }
}

/// Append a lone surrogate to `out` as valid WTF-8 bytes.
fn push_surrogate(out: &mut Vec<u8>, c: CodePoint) {
    let mut buf = Wtf8Buf::new();
    buf.push(c);
    out.extend_from_slice(buf.as_bytes());
}

fn capitalize_utf8(s: &str, out: &mut FmtWriter<'_>) {
    let mut chars = s.char_indices();
    if let Some((first_pos, first_ch)) = chars.next() {
        let first = &s[..first_pos + first_ch.len_utf8()];
        titlecase_segment(first, out);
    }
    for (i, ch) in chars {
        lowercase_or_sigma(ch, s, i, out);
    }
}

/// Title case a string following CPython conventions.
///
/// The first character of each run of cased characters is title cased and the
/// rest are lowercased; a new run starts after any non-cased character (digits,
/// whitespace, punctuation, etc.).
/// "123abc" -> "123Abc"
/// "123abc456def" -> "123Abc456Def"
/// "123 abc" -> "123 Abc"
fn titlecase_string(s: &str, out: &mut FmtWriter<'_>) {
    let mut previous_is_cased = false;
    for (i, ch) in s.char_indices() {
        if previous_is_cased {
            lowercase_or_sigma(ch, s, i, out);
        } else {
            titlecase_segment(&s[i..i + ch.len_utf8()], out);
        }

        previous_is_cased = is_cased(ch);
    }
}

fn titlecase_segment(s: &str, out: &mut FmtWriter<'_>) {
    // Callers pass a single first-of-word code point, which Python titlecases
    // unconditionally (applying its titlecase mapping). The default `Auto`
    // leading adjustment looks for a head in Letter/Number/Symbol/Private_Use
    // and skips anything else, dropping the titlecase mapping of cased marks
    // such as U+0345 (`ͅ`, general category Mn) -> U+0399 (`Ι`). `None`
    // titlecases the code point as given.
    let mut options = TitlecaseOptions::default();
    options.leading_adjustment = Some(LeadingAdjustment::None);
    TitlecaseMapper::new()
        .titlecase_segment(s, &LanguageIdentifier::UNKNOWN, options)
        .write_to(out)
        .expect("writing to an in-memory buffer cannot fail");
}

#[inline]
fn lowercase_or_sigma(ch: char, s: &str, i: usize, out: &mut FmtWriter<'_>) {
    let sigma = 'Σ';
    if ch.is_ascii() {
        push_char(ch.to_ascii_lowercase(), out);
    } else if ch == sigma {
        push_char(handle_capital_sigma(s, i), out);
    } else {
        let index = lookup_page(ch, LOWER_INDEX, LOWER_PAGES);
        push_mapping(ch, index, LOWER_MAPPINGS, |ch| push_char(ch, out));
    }
}

// Handle context-sensitive sigma.
//
// Character-by-character title/capitalize/swapcase mappings need the context
// from the original string, as in CPython's lower_ucs4.
fn handle_capital_sigma(s: &str, i: usize) -> char {
    let (left, rest) = s.split_at(i);
    let right = &rest['Σ'.len_utf8()..];

    // Check if any chars before or after sigma are cased.
    let before = left
        .chars()
        .rev()
        .find(|&ch| !is_case_ignorable(ch))
        .is_some_and(is_cased);
    let after = right
        .chars()
        .find(|&ch| !is_case_ignorable(ch))
        .is_some_and(is_cased);
    if before && !after { 'ς' } else { 'σ' }
}

fn swapcase_utf8(s: &str, out: &mut FmtWriter<'_>) {
    for (i, ch) in s.char_indices() {
        if ch.is_ascii() {
            let mapped = if ch.is_ascii_uppercase() {
                ch.to_ascii_lowercase()
            } else {
                ch.to_ascii_uppercase()
            };
            push_char(mapped, out);
        } else if is_uppercase(ch) {
            lowercase_or_sigma(ch, s, i, out);
        } else if is_lowercase(ch) {
            uppercase_char(ch, out);
        } else {
            push_char(ch, out);
        }
    }
}

#[inline]
fn uppercase_char(ch: char, out: &mut FmtWriter<'_>) {
    if ch.is_ascii() {
        push_char(ch.to_ascii_uppercase(), out);
    } else {
        let index = lookup_page(ch, UPPER_INDEX, UPPER_PAGES);
        push_mapping(ch, index, UPPER_MAPPINGS, |ch| push_char(ch, out));
    }
}

#[inline(always)]
fn push_mapping(ch: char, index: usize, mappings: &[[char; 3]], mut push: impl FnMut(char)) {
    if index == 0 {
        push(ch);
        return;
    }
    match mappings[index] {
        [a, '\0', _] => push(a),
        [a, b, '\0'] => {
            push(a);
            push(b);
        }
        [a, b, c] => {
            push(a);
            push(b);
            push(c);
        }
    }
}

#[inline]
fn push_char(ch: char, out: &mut FmtWriter<'_>) {
    if ch.is_ascii() {
        out.0.push(ch as u8);
        return;
    }
    let mut buf = [0u8; 4];
    out.0.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
}

/// Adapter so `icu`'s `Writeable` output can be appended to a byte buffer.
struct FmtWriter<'a>(&'a mut Vec<u8>);

impl core::fmt::Write for FmtWriter<'_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.0.extend_from_slice(s.as_bytes());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::{format, string::ToString};
    use rustpython_wtf8::{CodePoint, Wtf8Buf};

    use super::{
        capitalize_str, casefold_str, is_case_ignorable, is_cased, is_lowercase, is_titlecase,
        is_uppercase, lowercase_str, lowercase_wtf8, simple_lowercase, simple_uppercase,
        swapcase_str, title_str, title_wtf8, uppercase_str, uppercase_wtf8,
    };

    #[test]
    fn full_case_mappings_use_unicode_database() {
        // These characters acquired mappings in Unicode 18. Rust's Unicode
        // data can be newer than the Unicode 17 database used by Python 3.15.
        for c in ['\u{0277}', '\u{027c}', '\u{ab4b}', '\u{ab4c}'] {
            let s = c.to_string();
            assert_eq!(uppercase_str(&s), s);
            assert_eq!(swapcase_str(&s), s);
        }
        for c in ['\u{a7dd}', '\u{a7e2}', '\u{ab6c}', '\u{ab6d}'] {
            let s = c.to_string();
            assert_eq!(lowercase_str(&s), s);
            assert_eq!(capitalize_str(&format!("a{s}")), format!("A{s}"));
            assert_eq!(title_str(&format!("A{s}")), format!("A{s}"));
            assert_eq!(swapcase_str(&s), s);
            assert!(!is_uppercase(c));
        }
        assert_eq!(lowercase_str("ΟΔΟΣ"), "οδος");
        assert_eq!(lowercase_str("AΣ\u{0301}"), "aς\u{0301}");
        assert_eq!(lowercase_str("AΣ\u{0301}A"), "aσ\u{0301}a");
        assert_eq!(uppercase_str("ß"), "SS");
    }

    #[test]
    fn full_case_mappings_preserve_surrogates_and_sigma_boundaries() {
        let mut text = Wtf8Buf::from("AΣ");
        text.push(CodePoint::from_u32(0xd800).unwrap());
        text.push_str("ΣAß");
        let mut lower = Wtf8Buf::from("aς");
        lower.push(CodePoint::from_u32(0xd800).unwrap());
        lower.push_str("σaß");
        let mut upper = Wtf8Buf::from("AΣ");
        upper.push(CodePoint::from_u32(0xd800).unwrap());
        upper.push_str("\u{03a3}ASS");
        assert_eq!(lowercase_wtf8(&text), lower);
        assert_eq!(uppercase_wtf8(&text), upper);
    }

    #[test]
    fn casefold_full_mappings() {
        // ß case-folds to "ss"
        assert_eq!(casefold_str("ß"), "ss");
        assert_eq!(casefold_str("Σ"), "σ");
    }

    #[test]
    fn simple_mappings_are_one_to_one() {
        // ß has no simple uppercase mapping, so it stays unchanged (unlike the
        // full mapping "SS").
        assert_eq!(simple_uppercase('ß'), 'ß');
        assert_eq!(simple_uppercase('a'), 'A');
        assert_eq!(simple_lowercase('A'), 'a');
        // ǅ (U+01C5) simple-titlecases to itself but upper/lowercases away.
        assert_eq!(simple_uppercase('ǅ'), 'Ǆ');
        assert_eq!(simple_lowercase('ǅ'), 'ǆ');
    }

    #[test]
    fn generated_case_data_matches_icu() {
        use icu_properties::props::{BinaryProperty, CaseIgnorable, Cased, Lowercase, Uppercase};
        let mapper = icu_casemap::CaseMapper::new();
        let locale = icu_locale::LanguageIdentifier::UNKNOWN;
        for ch in (0..=0x10ffff).filter_map(char::from_u32) {
            let mut buf = [0; 4];
            let text = ch.encode_utf8(&mut buf);
            assert_eq!(
                lowercase_str(text),
                mapper.lowercase_to_string(text, &locale),
                "{ch:?}"
            );
            assert_eq!(
                uppercase_str(text),
                mapper.uppercase_to_string(text, &locale),
                "{ch:?}"
            );
            assert_eq!(is_cased(ch), Cased::for_char(ch), "{ch:?}");
            assert_eq!(is_lowercase(ch), Lowercase::for_char(ch), "{ch:?}");
            assert_eq!(is_uppercase(ch), Uppercase::for_char(ch), "{ch:?}");
            assert_eq!(is_case_ignorable(ch), CaseIgnorable::for_char(ch), "{ch:?}");
        }
    }

    #[test]
    fn casing_predicates() {
        assert!(is_lowercase('a'));
        assert!(!is_lowercase('A'));
        assert!(is_uppercase('A'));
        assert!(is_titlecase('ǅ'));
        assert!(!is_titlecase('D'));
        assert!(is_cased('a') && is_cased('A'));
        assert!(!is_cased('1'));
        assert!(is_case_ignorable('\''));
        assert!(!is_case_ignorable('a'));
    }

    #[test]
    fn capitalize_final_sigma() {
        // Final sigma at end of a cased run becomes ς.
        assert_eq!(capitalize_str("ΟΔΟΣ"), "Οδος");
        assert_eq!(title_str("hello world"), "Hello World");
        assert_eq!(swapcase_str("Hello"), "hELLO");
    }

    #[test]
    fn titlecase_first_of_word_takes_titlecase_mapping() {
        // A leading cased combining mark still takes its titlecase mapping:
        // U+0345 (ͅ, general category Mn) titlecases to U+0399 (Ι), even though
        // it is not a Letter/Number/Symbol head.
        assert_eq!(title_str("\u{0345}"), "\u{0399}");
        assert_eq!(capitalize_str("\u{0345}"), "\u{0399}");
        assert_eq!(title_str("\u{0345}a"), "\u{0399}a");
        // Full (one-to-many) titlecase mappings still apply to the first
        // character of each word.
        // cspell:ignore ﬁnnish NNISH ǳungla ǲungla ßhello Sshello
        assert_eq!(capitalize_str("ﬁnnish"), "Finnish");
        assert_eq!(title_str("ﬁNNISH"), "Finnish");
        assert_eq!(capitalize_str("ǳungla"), "ǲungla");
        assert_eq!(capitalize_str("ßhello"), "Sshello");
    }

    #[test]
    fn ascii_fast_path_matches_icu() {
        let mapper = icu_casemap::CaseMapper::new();
        for c in (0u8..0x80).map(char::from) {
            assert_eq!(simple_lowercase(c), mapper.simple_lowercase(c));
            assert_eq!(simple_uppercase(c), mapper.simple_uppercase(c));
        }
    }

    #[test]
    fn wtf8_passes_surrogates_through() {
        let mut buf = Wtf8Buf::from("ab cd");
        buf.push(CodePoint::from_u32(0xD800).unwrap());
        let titled = title_wtf8(&buf);
        assert!(titled.code_points().any(|c| c.to_u32() == 0xD800));
        assert_eq!(
            titled.code_points().next().and_then(|c| c.to_char()),
            Some('A')
        );
    }
}
