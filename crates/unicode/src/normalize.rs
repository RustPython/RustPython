//! Unicode normalization (`unicodedata.normalize` / `is_normalized`).

// spell-checker:ignore nfkc

use alloc::vec::Vec;
use core::str::FromStr;

use icu_normalizer::properties::CanonicalComposition;
use icu_normalizer::{ComposingNormalizerBorrowed, DecomposingNormalizerBorrowed};
use icu_properties::props::{CanonicalCombiningClass, EnumeratedProperty};
use rustpython_wtf8::{CodePoint, Wtf8, Wtf8Buf, Wtf8Chunk};

use crate::Ucd;

#[cfg(feature = "ucd_3_2_0")]
include!(concat!(env!("OUT_DIR"), "/generated/normalization_3_2.rs"));
#[cfg(not(feature = "ucd_3_2_0"))]
const NORMALIZATION_3_2: &[(u32, u32)] = &[];

/// One of the four Unicode normalization forms.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum NormalizeForm {
    Nfc,
    Nfkc,
    Nfd,
    Nfkd,
}

impl FromStr for NormalizeForm {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "NFC" => Ok(Self::Nfc),
            "NFKC" => Ok(Self::Nfkc),
            "NFD" => Ok(Self::Nfd),
            "NFKD" => Ok(Self::Nfkd),
            _ => Err(()),
        }
    }
}

/// Normalize `text` to `form` (`unicodedata.normalize`).
///
/// Lone surrogates are passed through unchanged; only the valid UTF-8 runs are
/// normalized.
#[must_use]
pub fn normalize(form: NormalizeForm, text: &Wtf8) -> Wtf8Buf {
    match form {
        NormalizeForm::Nfc => {
            let normalizer = ComposingNormalizerBorrowed::new_nfc();
            text.map_utf8(|s| normalizer.normalize_iter(s.chars()))
                .collect()
        }
        NormalizeForm::Nfkc => {
            let normalizer = ComposingNormalizerBorrowed::new_nfkc();
            text.map_utf8(|s| normalizer.normalize_iter(s.chars()))
                .collect()
        }
        NormalizeForm::Nfd => {
            let normalizer = DecomposingNormalizerBorrowed::new_nfd();
            text.map_utf8(|s| normalizer.normalize_iter(s.chars()))
                .collect()
        }
        NormalizeForm::Nfkd => {
            let normalizer = DecomposingNormalizerBorrowed::new_nfkd();
            text.map_utf8(|s| normalizer.normalize_iter(s.chars()))
                .collect()
        }
    }
}

/// Whether `text` is already in `form` (`unicodedata.is_normalized`).
///
/// Lone surrogates split the text into valid UTF-8 runs; each run is checked
/// independently, matching the run-wise normalization performed by [`normalize`].
#[must_use]
pub fn is_normalized(form: NormalizeForm, text: &Wtf8) -> bool {
    let check: fn(&str) -> bool = match form {
        NormalizeForm::Nfc => |s| ComposingNormalizerBorrowed::new_nfc().is_normalized(s),
        NormalizeForm::Nfkc => |s| ComposingNormalizerBorrowed::new_nfkc().is_normalized(s),
        NormalizeForm::Nfd => |s| DecomposingNormalizerBorrowed::new_nfd().is_normalized(s),
        NormalizeForm::Nfkd => |s| DecomposingNormalizerBorrowed::new_nfkd().is_normalized(s),
    };
    text.chunks().all(|chunk| match chunk {
        Wtf8Chunk::Utf8(s) => check(s),
        Wtf8Chunk::Surrogate(_) => true,
    })
}

impl Ucd {
    /// Normalize using this database version, including Unicode 3.2 corrections.
    #[must_use]
    pub fn normalize(self, form: NormalizeForm, text: &Wtf8) -> Wtf8Buf {
        if self.modern {
            return normalize(form, text);
        }
        let decomposer = match form {
            NormalizeForm::Nfc | NormalizeForm::Nfd => DecomposingNormalizerBorrowed::new_nfd(),
            NormalizeForm::Nfkc | NormalizeForm::Nfkd => DecomposingNormalizerBorrowed::new_nfkd(),
        };
        let mut decomposed = Vec::new();
        for code in text.code_points() {
            match code.to_char() {
                Some(ch) if self.membership(ch) => {
                    let ch = NORMALIZATION_3_2
                        .binary_search_by_key(&(ch as u32), |&(code, _)| code)
                        .ok()
                        .map_or(ch, |i| char::from_u32(NORMALIZATION_3_2[i].1).unwrap());
                    decomposed.extend(
                        decomposer
                            .normalize_iter(core::iter::once(ch))
                            .map(CodePoint::from),
                    );
                }
                // CPython does not decompose characters unassigned in 3.2.
                _ => decomposed.push(code),
            }
        }

        // CPython uses current combining classes for ordering and composition,
        // even for characters that were unassigned in the old database.
        let combining = |code: &CodePoint| {
            code.to_char().map_or(0, |ch| {
                CanonicalCombiningClass::for_char(ch).to_icu4c_value()
            })
        };
        let mut start = 0;
        for i in 0..=decomposed.len() {
            if i == decomposed.len() || combining(&decomposed[i]) == 0 {
                decomposed[start..i].sort_by_key(&combining);
                start = i + 1;
            }
        }
        if matches!(form, NormalizeForm::Nfc | NormalizeForm::Nfkc) {
            let composer = CanonicalComposition::new();
            let mut output: Vec<CodePoint> = Vec::with_capacity(decomposed.len());
            let mut starter: Option<usize> = None;
            let mut previous_class = 0;
            for code in decomposed {
                let class = combining(&code);
                if let Some(i) = starter
                    && (previous_class == 0 || previous_class < class)
                    && let (Some(first), Some(second)) = (output[i].to_char(), code.to_char())
                    && let Some(composed) = composer.compose(first, second)
                {
                    output[i] = CodePoint::from(composed);
                    continue;
                }
                if class == 0 {
                    starter = Some(output.len());
                }
                previous_class = class;
                output.push(code);
            }
            output.into_iter().collect()
        } else {
            decomposed.into_iter().collect()
        }
    }

    /// Check normalization with the same version-specific decomposition rules.
    #[must_use]
    pub fn is_normalized(self, form: NormalizeForm, text: &Wtf8) -> bool {
        if self.modern {
            is_normalized(form, text)
        } else {
            // CPython also disables normalization quick checks for old UCDs.
            self.normalize(form, text).as_slice() == text
        }
    }
}

#[cfg(test)]
mod tests {
    use rustpython_wtf8::{CodePoint, Wtf8Buf};

    use super::{NormalizeForm, is_normalized, normalize};

    #[test]
    #[cfg(feature = "ucd_3_2_0")]
    fn old_normalization_keeps_current_ordering_and_composition() {
        let old = crate::Ucd::new(false);
        for (form, text, expected) in [
            (NormalizeForm::Nfd, "a\u{1ab0}\u{0327}", "a\u{0327}\u{1ab0}"),
            (NormalizeForm::Nfc, "\u{1b05}\u{1b35}", "\u{1b06}"),
            (NormalizeForm::Nfc, "A\u{1ab0}\u{0300}", "A\u{1ab0}\u{0300}"),
            (NormalizeForm::Nfc, "\u{09c7}\u{09be}", "\u{09cb}"),
            (NormalizeForm::Nfc, "\u{1100}\u{1161}\u{11a8}", "\u{ac01}"),
            (NormalizeForm::Nfkd, "\u{1d2c}", "\u{1d2c}"),
            (NormalizeForm::Nfd, "\u{2f868}", "\u{2136a}"),
        ] {
            let text = Wtf8Buf::from(text);
            let expected = Wtf8Buf::from(expected);
            assert_eq!(old.normalize(form, &text), expected);
            assert_eq!(old.is_normalized(form, &text), text == expected);
        }
        let mut text = Wtf8Buf::from("\u{2f868}");
        text.push(CodePoint::from_u32(0xd800).unwrap());
        text.push_str("A\u{0300}");
        let mut expected = Wtf8Buf::from("\u{2136a}");
        expected.push(CodePoint::from_u32(0xd800).unwrap());
        expected.push_str("À");
        assert_eq!(old.normalize(NormalizeForm::Nfc, &text), expected);
    }

    #[test]
    fn normalization_round_trips() {
        let composed = Wtf8Buf::from("é");
        let decomposed = normalize(NormalizeForm::Nfd, &composed);
        assert_eq!(normalize(NormalizeForm::Nfc, &decomposed), composed);
        assert!(is_normalized(
            NormalizeForm::Nfc,
            Wtf8Buf::from("é").as_ref()
        ));
        assert!(!is_normalized(
            NormalizeForm::Nfd,
            Wtf8Buf::from("é").as_ref()
        ));
    }

    #[test]
    fn is_normalized_skips_lone_surrogates() {
        // A lone surrogate splits the text into UTF-8 runs; each run is checked
        // independently, so a surrogate next to normalized text stays normalized.
        let mut buf = Wtf8Buf::from("é");
        buf.push(CodePoint::from_u32(0xD800).unwrap());
        assert!(is_normalized(NormalizeForm::Nfc, &buf));
        assert!(!is_normalized(NormalizeForm::Nfd, &buf));
    }
}
