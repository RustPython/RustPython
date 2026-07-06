//! Unicode normalization (`unicodedata.normalize` / `is_normalized`).

// spell-checker:ignore nfkc

use core::str::FromStr;

use icu_normalizer::{ComposingNormalizerBorrowed, DecomposingNormalizerBorrowed};
use rustpython_wtf8::{Wtf8, Wtf8Buf};

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

/// Whether `bytes` (interpreted as UTF-8) is already in `form`
/// (`unicodedata.is_normalized`).
#[must_use]
pub fn is_normalized(form: NormalizeForm, bytes: &[u8]) -> bool {
    match form {
        NormalizeForm::Nfc => ComposingNormalizerBorrowed::new_nfc().is_normalized_utf8(bytes),
        NormalizeForm::Nfkc => ComposingNormalizerBorrowed::new_nfkc().is_normalized_utf8(bytes),
        NormalizeForm::Nfd => DecomposingNormalizerBorrowed::new_nfd().is_normalized_utf8(bytes),
        NormalizeForm::Nfkd => DecomposingNormalizerBorrowed::new_nfkd().is_normalized_utf8(bytes),
    }
}

#[cfg(test)]
mod tests {
    use rustpython_wtf8::Wtf8Buf;

    use super::{NormalizeForm, is_normalized, normalize};

    #[test]
    fn normalization_round_trips() {
        let composed = Wtf8Buf::from("é");
        let decomposed = normalize(NormalizeForm::Nfd, &composed);
        assert_eq!(normalize(NormalizeForm::Nfc, &decomposed), composed);
        assert!(is_normalized(NormalizeForm::Nfc, "é".as_bytes()));
        assert!(!is_normalized(NormalizeForm::Nfd, "é".as_bytes()));
    }
}
