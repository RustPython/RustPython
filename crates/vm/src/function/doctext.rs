//! A docstring is a static literal, a span of the compressed blob, or both.
//! `len == 0` means there is no database body. `offset == u32::MAX` is an
//! explicit empty entry.

#[cfg(feature = "doc")]
use std::sync::OnceLock;

/// Static text plus an optional span of the compressed doc blob.
///
/// `text` is a plain docstring, a full `name(sig)\n--\n\nbody` literal, or only
/// the signature prefix when `len` is the database body.
#[derive(Clone, Copy, Debug)]
pub struct ItemDoc {
    pub text: Option<&'static str>,
    pub offset: u32,
    pub len: u32,
}

impl ItemDoc {
    pub const NONE: Self = Self {
        text: None,
        offset: 0,
        len: 0,
    };

    #[must_use]
    pub const fn static_text(text: &'static str) -> Self {
        Self {
            text: Some(text),
            offset: 0,
            len: 0,
        }
    }

    #[must_use]
    pub const fn is_db(self) -> bool {
        self.len != 0
    }
}

impl Default for ItemDoc {
    fn default() -> Self {
        Self::NONE
    }
}

#[inline(never)]
#[must_use]
pub fn db_doc(offset: u32, len: u32) -> Option<&'static str> {
    if len == 0 {
        return None;
    }
    db_slice(offset, len)
}

#[inline(never)]
#[must_use]
pub fn plain_doc(doc: ItemDoc) -> Option<&'static str> {
    if doc.len != 0 {
        return db_doc(doc.offset, doc.len);
    }
    doc.text.filter(|text| !text.is_empty())
}

#[cfg(feature = "doc")]
#[inline(never)]
fn db_slice(offset: u32, len: u32) -> Option<&'static str> {
    let text = docs();
    let start = offset as usize;
    let end = start.checked_add(len as usize)?;
    text.get(start..end)
}

#[cfg(not(feature = "doc"))]
fn db_slice(_offset: u32, _len: u32) -> Option<&'static str> {
    None
}

#[cfg(feature = "doc")]
#[inline(never)]
fn docs() -> &'static str {
    static TEXT: OnceLock<Box<str>> = OnceLock::new();
    TEXT.get_or_init(|| {
        let bytes = match xz::decode_all(rustpython_doc::BLOB) {
            Ok(bytes) => bytes,
            Err(_) => panic!("doc blob"),
        };
        match String::from_utf8(bytes) {
            Ok(text) => text.into_boxed_str(),
            Err(_) => panic!("doc blob"),
        }
    })
    .as_ref()
}
