//! Case folding for Python `str.casefold`.
//!
//! Lower, upper, and title casing of `str` objects stay with the runtime
//! because they iterate the string with special final-sigma handling. Case
//! folding has no such context dependence, so it lives here and is shared with
//! other runtimes.

use alloc::{
    string::{String, ToString},
    vec::Vec,
};

use icu_casemap::CaseMapper;
use rustpython_wtf8::{Wtf8, Wtf8Buf, Wtf8Chunk};
use writeable::Writeable;

/// Full Unicode case fold of `text` (`str.casefold`).
#[must_use]
pub fn casefold_str(text: &str) -> String {
    CaseMapper::new().fold_string(text).to_string()
}

/// Full Unicode case fold of `text`, passing lone surrogates through unchanged.
#[must_use]
pub fn casefold_wtf8(text: &Wtf8) -> Wtf8Buf {
    let mut out = Vec::with_capacity(text.len());
    let mapper = CaseMapper::new();
    for chunk in text.chunks() {
        match chunk {
            Wtf8Chunk::Utf8(s) => {
                mapper
                    .fold(s)
                    .write_to(&mut FmtWriter(&mut out))
                    .expect("writing to an in-memory buffer cannot fail");
            }
            Wtf8Chunk::Surrogate(c) => {
                let mut buf = Wtf8Buf::new();
                buf.push(c);
                out.extend_from_slice(buf.as_bytes());
            }
        }
    }
    // SAFETY:
    // * CaseMapper only produces valid UTF-8.
    // * Surrogates are appended as valid WTF-8 (encoded via Wtf8Buf::push).
    unsafe { Wtf8Buf::from_bytes_unchecked(out) }
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
    use super::casefold_str;

    #[test]
    fn casefold_full_mappings() {
        // ß case-folds to "ss"
        assert_eq!(casefold_str("ß"), "ss");
        assert_eq!(casefold_str("Σ"), "σ");
    }
}
