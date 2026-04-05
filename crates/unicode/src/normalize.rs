use core::str::FromStr;
use icu_normalizer::{ComposingNormalizerBorrowed, DecomposingNormalizerBorrowed};
use rustpython_wtf8::{Wtf8, Wtf8Buf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

pub fn is_normalized(form: NormalizeForm, text: &Wtf8) -> bool {
    let normalized = normalize(form, text);
    text == &*normalized
}
