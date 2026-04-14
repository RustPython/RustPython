#[cfg(feature = "casefold")]
use alloc::string::String;

#[cfg(feature = "casefold")]
use rustpython_wtf8::Wtf8Chunk;
use rustpython_wtf8::{Wtf8, Wtf8Buf};
use unicode_casing::CharExt;

use crate::char_from_codepoint;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaseMapping {
    len: u8,
    codepoints: [u32; 3],
}

impl CaseMapping {
    pub const fn identity(cp: u32) -> Self {
        Self {
            len: 1,
            codepoints: [cp, 0, 0],
        }
    }

    pub const fn first(self) -> Option<u32> {
        if self.len == 0 {
            None
        } else {
            Some(self.codepoints[0])
        }
    }

    pub fn iter(self) -> impl Iterator<Item = u32> {
        self.codepoints.into_iter().take(usize::from(self.len))
    }
}

fn mapping_from_chars(chars: impl Iterator<Item = char>) -> CaseMapping {
    let mut codepoints = [0; 3];
    let mut len = 0;
    for ch in chars.take(codepoints.len()) {
        codepoints[len] = ch as u32;
        len += 1;
    }
    CaseMapping {
        len: len as u8,
        codepoints,
    }
}

#[cfg(feature = "casefold")]
fn mapping_from_string(text: String) -> CaseMapping {
    mapping_from_chars(text.chars())
}

pub fn to_lowercase(cp: u32) -> CaseMapping {
    char_from_codepoint(cp).map_or_else(
        || CaseMapping::identity(cp),
        |ch| mapping_from_chars(ch.to_lowercase()),
    )
}

pub fn to_uppercase(cp: u32) -> CaseMapping {
    char_from_codepoint(cp).map_or_else(
        || CaseMapping::identity(cp),
        |ch| mapping_from_chars(ch.to_uppercase()),
    )
}

pub fn to_titlecase(cp: u32) -> CaseMapping {
    char_from_codepoint(cp).map_or_else(
        || CaseMapping::identity(cp),
        |ch| mapping_from_chars(ch.to_titlecase()),
    )
}

pub fn to_lowercase_wtf8(text: &Wtf8) -> Wtf8Buf {
    text.map_utf8(|s| s.chars().flat_map(char::to_lowercase))
        .collect()
}

pub fn to_uppercase_wtf8(text: &Wtf8) -> Wtf8Buf {
    text.map_utf8(|s| s.chars().flat_map(char::to_uppercase))
        .collect()
}

#[cfg(feature = "casefold")]
pub fn casefold(cp: u32) -> CaseMapping {
    char_from_codepoint(cp).map_or_else(
        || CaseMapping::identity(cp),
        |ch| {
            let mut buf = [0; 4];
            mapping_from_string(caseless::default_case_fold_str(ch.encode_utf8(&mut buf)))
        },
    )
}

#[cfg(feature = "casefold")]
pub fn casefold_str(text: &str) -> String {
    caseless::default_case_fold_str(text)
}

#[cfg(feature = "casefold")]
pub fn casefold_wtf8(text: &Wtf8) -> Wtf8Buf {
    text.chunks()
        .map(|chunk| match chunk {
            Wtf8Chunk::Utf8(s) => Wtf8Buf::from_string(casefold_str(s)),
            Wtf8Chunk::Surrogate(c) => Wtf8Buf::from(c),
        })
        .collect()
}
