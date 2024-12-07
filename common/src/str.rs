use crate::{
    atomic::{PyAtomic, Radium},
    hash::PyHash,
};
use ascii::AsciiString;
use rustpython_format::CharLen;
use std::ops::{Bound, RangeBounds};

#[cfg(not(target_arch = "wasm32"))]
#[allow(non_camel_case_types)]
pub type wchar_t = libc::wchar_t;
#[cfg(target_arch = "wasm32")]
#[allow(non_camel_case_types)]
pub type wchar_t = u32;

/// Utf8 + state.ascii (+ PyUnicode_Kind in future)
#[derive(Debug, Copy, Clone, PartialEq)]
pub enum PyStrKind {
    Ascii,
    Utf8,
}

impl std::ops::BitOr for PyStrKind {
    type Output = Self;
    fn bitor(self, other: Self) -> Self {
        match (self, other) {
            (Self::Ascii, Self::Ascii) => Self::Ascii,
            _ => Self::Utf8,
        }
    }
}

impl PyStrKind {
    #[inline]
    pub fn new_data(self) -> PyStrKindData {
        match self {
            PyStrKind::Ascii => PyStrKindData::Ascii,
            PyStrKind::Utf8 => PyStrKindData::Utf8(Radium::new(usize::MAX)),
        }
    }
}

#[derive(Debug)]
pub enum PyStrKindData {
    Ascii,
    // uses usize::MAX as a sentinel for "uncomputed"
    Utf8(PyAtomic<usize>),
}

impl PyStrKindData {
    #[inline]
    pub fn kind(&self) -> PyStrKind {
        match self {
            PyStrKindData::Ascii => PyStrKind::Ascii,
            PyStrKindData::Utf8(_) => PyStrKind::Utf8,
        }
    }
}

pub struct BorrowedStr<'a> {
    bytes: &'a [u8],
    kind: PyStrKindData,
    #[allow(dead_code)]
    hash: PyAtomic<PyHash>,
}

impl<'a> BorrowedStr<'a> {
    /// # Safety
    /// `s` have to be an ascii string
    #[inline]
    pub unsafe fn from_ascii_unchecked(s: &'a [u8]) -> Self {
        debug_assert!(s.is_ascii());
        Self {
            bytes: s,
            kind: PyStrKind::Ascii.new_data(),
            hash: PyAtomic::<PyHash>::new(0),
        }
    }

    #[inline]
    pub fn from_bytes(s: &'a [u8]) -> Self {
        let k = if s.is_ascii() {
            PyStrKind::Ascii.new_data()
        } else {
            PyStrKind::Utf8.new_data()
        };
        Self {
            bytes: s,
            kind: k,
            hash: PyAtomic::<PyHash>::new(0),
        }
    }

    #[inline]
    pub fn as_str(&self) -> &str {
        unsafe {
            // SAFETY: Both PyStrKind::{Ascii, Utf8} are valid utf8 string
            std::str::from_utf8_unchecked(self.bytes)
        }
    }

    #[inline]
    pub fn char_len(&self) -> usize {
        match self.kind {
            PyStrKindData::Ascii => self.bytes.len(),
            PyStrKindData::Utf8(ref len) => match len.load(core::sync::atomic::Ordering::Relaxed) {
                usize::MAX => self._compute_char_len(),
                len => len,
            },
        }
    }

    #[cold]
    fn _compute_char_len(&self) -> usize {
        match self.kind {
            PyStrKindData::Utf8(ref char_len) => {
                let len = self.as_str().chars().count();
                // len cannot be usize::MAX, since vec.capacity() < sys.maxsize
                char_len.store(len, core::sync::atomic::Ordering::Relaxed);
                len
            }
            _ => unsafe {
                debug_assert!(false); // invalid for non-utf8 strings
                std::hint::unreachable_unchecked()
            },
        }
    }
}

impl std::ops::Deref for BorrowedStr<'_> {
    type Target = str;
    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for BorrowedStr<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.as_str().fmt(f)
    }
}

impl CharLen for BorrowedStr<'_> {
    fn char_len(&self) -> usize {
        self.char_len()
    }
}

pub fn try_get_chars(s: &str, range: impl RangeBounds<usize>) -> Option<&str> {
    let mut chars = s.chars();
    let start = match range.start_bound() {
        Bound::Included(&i) => i,
        Bound::Excluded(&i) => i + 1,
        Bound::Unbounded => 0,
    };
    for _ in 0..start {
        chars.next()?;
    }
    let s = chars.as_str();
    let range_len = match range.end_bound() {
        Bound::Included(&i) => i + 1 - start,
        Bound::Excluded(&i) => i - start,
        Bound::Unbounded => return Some(s),
    };
    char_range_end(s, range_len).map(|end| &s[..end])
}

pub fn get_chars(s: &str, range: impl RangeBounds<usize>) -> &str {
    try_get_chars(s, range).unwrap()
}

#[inline]
pub fn char_range_end(s: &str, nchars: usize) -> Option<usize> {
    let i = match nchars.checked_sub(1) {
        Some(last_char_index) => {
            let (index, c) = s.char_indices().nth(last_char_index)?;
            index + c.len_utf8()
        }
        None => 0,
    };
    Some(i)
}

pub fn zfill(bytes: &[u8], width: usize) -> Vec<u8> {
    if width <= bytes.len() {
        bytes.to_vec()
    } else {
        let (sign, s) = match bytes.first() {
            Some(_sign @ b'+') | Some(_sign @ b'-') => {
                (unsafe { bytes.get_unchecked(..1) }, &bytes[1..])
            }
            _ => (&b""[..], bytes),
        };
        let mut filled = Vec::new();
        filled.extend_from_slice(sign);
        filled.extend(std::iter::repeat(b'0').take(width - bytes.len()));
        filled.extend_from_slice(s);
        filled
    }
}

/// Convert a string to ascii compatible, escaping unicodes into escape
/// sequences.
pub fn to_ascii(value: &str) -> AsciiString {
    let mut ascii = Vec::new();
    for c in value.chars() {
        if c.is_ascii() {
            ascii.push(c as u8);
        } else {
            let c = c as i64;
            let hex = if c < 0x100 {
                format!("\\x{c:02x}")
            } else if c < 0x10000 {
                format!("\\u{c:04x}")
            } else {
                format!("\\U{c:08x}")
            };
            ascii.append(&mut hex.into_bytes());
        }
    }
    unsafe { AsciiString::from_ascii_unchecked(ascii) }
}

pub mod levenshtein {
    use std::{cell::RefCell, thread_local};

    pub const MOVE_COST: usize = 2;
    const CASE_COST: usize = 1;
    const MAX_STRING_SIZE: usize = 40;

    fn substitution_cost(mut a: u8, mut b: u8) -> usize {
        if (a & 31) != (b & 31) {
            return MOVE_COST;
        }
        if a == b {
            return 0;
        }
        if a.is_ascii_uppercase() {
            a += b'a' - b'A';
        }
        if b.is_ascii_uppercase() {
            b += b'a' - b'A';
        }
        if a == b {
            CASE_COST
        } else {
            MOVE_COST
        }
    }

    pub fn levenshtein_distance(a: &str, b: &str, max_cost: usize) -> usize {
        thread_local! {
            static BUFFER: RefCell<[usize; MAX_STRING_SIZE]> = const { RefCell::new([0usize; MAX_STRING_SIZE]) };
        }

        if a == b {
            return 0;
        }

        let (mut a_bytes, mut b_bytes) = (a.as_bytes(), b.as_bytes());
        let (mut a_begin, mut a_end) = (0usize, a.len());
        let (mut b_begin, mut b_end) = (0usize, b.len());

        while a_end > 0 && b_end > 0 && (a_bytes[a_begin] == b_bytes[b_begin]) {
            a_begin += 1;
            b_begin += 1;
            a_end -= 1;
            b_end -= 1;
        }
        while a_end > 0
            && b_end > 0
            && (a_bytes[a_begin + a_end - 1] == b_bytes[b_begin + b_end - 1])
        {
            a_end -= 1;
            b_end -= 1;
        }
        if a_end == 0 || b_end == 0 {
            return (a_end + b_end) * MOVE_COST;
        }
        if a_end > MAX_STRING_SIZE || b_end > MAX_STRING_SIZE {
            return max_cost + 1;
        }

        if b_end < a_end {
            std::mem::swap(&mut a_bytes, &mut b_bytes);
            std::mem::swap(&mut a_begin, &mut b_begin);
            std::mem::swap(&mut a_end, &mut b_end);
        }

        if (b_end - a_end) * MOVE_COST > max_cost {
            return max_cost + 1;
        }

        BUFFER.with(|buffer| {
            let mut buffer = buffer.borrow_mut();
            for i in 0..a_end {
                buffer[i] = (i + 1) * MOVE_COST;
            }

            let mut result = 0usize;
            for (b_index, b_code) in b_bytes[b_begin..(b_begin + b_end)].iter().enumerate() {
                result = b_index * MOVE_COST;
                let mut distance = result;
                let mut minimum = usize::MAX;
                for (a_index, a_code) in a_bytes[a_begin..(a_begin + a_end)].iter().enumerate() {
                    let substitute = distance + substitution_cost(*b_code, *a_code);
                    distance = buffer[a_index];
                    let insert_delete = usize::min(result, distance) + MOVE_COST;
                    result = usize::min(insert_delete, substitute);

                    buffer[a_index] = result;
                    if result < minimum {
                        minimum = result;
                    }
                }
                if minimum > max_cost {
                    return max_cost + 1;
                }
            }
            result
        })
    }
}

/// Creates an [`AsciiStr`][ascii::AsciiStr] from a string literal, throwing a compile error if the
/// literal isn't actually ascii.
///
/// ```compile_fail
/// # use rustpython_common::str::ascii;
/// ascii!("I ❤️ Rust & Python");
/// ```
#[macro_export]
macro_rules! ascii {
    ($x:expr $(,)?) => {{
        let s = const {
            let s: &str = $x;
            assert!(s.is_ascii(), "ascii!() argument is not an ascii string");
            s
        };
        unsafe { $crate::vendored::ascii::AsciiStr::from_ascii_unchecked(s.as_bytes()) }
    }};
}
pub use ascii;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_chars() {
        let s = "0123456789";
        assert_eq!(get_chars(s, 3..7), "3456");
        assert_eq!(get_chars(s, 3..7), &s[3..7]);

        let s = "0유니코드 문자열9";
        assert_eq!(get_chars(s, 3..7), "코드 문");

        let s = "0😀😃😄😁😆😅😂🤣9";
        assert_eq!(get_chars(s, 3..7), "😄😁😆😅");
    }
}
