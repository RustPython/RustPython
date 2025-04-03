//! An implementation of [WTF-8], a utf8-compatible encoding that allows for
//! unpaired surrogate codepoints. This implementation additionally allows for
//! paired surrogates that are nonetheless treated as two separate codepoints.
//!
//!
//! RustPython uses this because CPython internally uses a variant of UCS-1/2/4
//! as its string storage, which treats each `u8`/`u16`/`u32` value (depending
//! on the highest codepoint value in the string) as simply integers, unlike
//! UTF-8 or UTF-16 where some characters are encoded using multi-byte
//! sequences. CPython additionally doesn't disallow the use of surrogates in
//! `str`s (which in UTF-16 pair together to represent codepoints with a value
//! higher than `u16::MAX`) and in fact takes quite extensive advantage of the
//! fact that they're allowed. The `surrogateescape` codec-error handler uses
//! them to represent byte sequences which are invalid in the given codec (e.g.
//! bytes with their high bit set in ASCII or UTF-8) by mapping them into the
//! surrogate range. `surrogateescape` is the default error handler in Python
//! for interacting with the filesystem, and thus if RustPython is to properly
//! support `surrogateescape`, its `str`s must be able to represent surrogates.
//!
//! We use WTF-8 over something more similar to CPython's string implementation
//! because of its compatibility with UTF-8, meaning that in the case where a
//! string has no surrogates, it can be viewed as a UTF-8 Rust [`prim@str`] without
//! needing any copies or re-encoding.
//!
//! This implementation is mostly copied from the WTF-8 implementation in the
//! Rust 1.85 standard library, which is used as the backing for [`OsStr`] on
//! Windows targets. As previously mentioned, however, it is modified to not
//! join two surrogates into one codepoint when concatenating strings, in order
//! to match CPython's behavior.
//!
//! [WTF-8]: https://simonsapin.github.io/wtf-8
//! [`OsStr`]: std::ffi::OsStr

#![allow(clippy::precedence, clippy::match_overlapping_arm)]

use core::fmt;
use core::hash::{Hash, Hasher};
use core::iter::FusedIterator;
use core::mem;
use core::ops;
use core::slice;
use core::str;
use core_char::MAX_LEN_UTF8;
use core_char::{MAX_LEN_UTF16, encode_utf8_raw, encode_utf16_raw, len_utf8};
use core_str::{next_code_point, next_code_point_reverse};
use itertools::{Either, Itertools};
use std::borrow::{Borrow, Cow};
use std::collections::TryReserveError;
use std::string::String;
use std::vec::Vec;

use bstr::{ByteSlice, ByteVec};

mod core_char;
mod core_str;
mod core_str_count;

const UTF8_REPLACEMENT_CHARACTER: &str = "\u{FFFD}";

/// A Unicode code point: from U+0000 to U+10FFFF.
///
/// Compares with the `char` type,
/// which represents a Unicode scalar value:
/// a code point that is not a surrogate (U+D800 to U+DFFF).
#[derive(Eq, PartialEq, Ord, PartialOrd, Clone, Copy)]
pub struct CodePoint {
    value: u32,
}

/// Format the code point as `U+` followed by four to six hexadecimal digits.
/// Example: `U+1F4A9`
impl fmt::Debug for CodePoint {
    #[inline]
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "U+{:04X}", self.value)
    }
}

impl fmt::Display for CodePoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.to_char_lossy().fmt(f)
    }
}

impl CodePoint {
    /// Unsafely creates a new `CodePoint` without checking the value.
    ///
    /// # Safety
    ///
    /// `value` must be less than or equal to 0x10FFFF.
    #[inline]
    pub unsafe fn from_u32_unchecked(value: u32) -> CodePoint {
        CodePoint { value }
    }

    /// Creates a new `CodePoint` if the value is a valid code point.
    ///
    /// Returns `None` if `value` is above 0x10FFFF.
    #[inline]
    pub fn from_u32(value: u32) -> Option<CodePoint> {
        match value {
            0..=0x10FFFF => Some(CodePoint { value }),
            _ => None,
        }
    }

    /// Creates a new `CodePoint` from a `char`.
    ///
    /// Since all Unicode scalar values are code points, this always succeeds.
    #[inline]
    pub fn from_char(value: char) -> CodePoint {
        CodePoint {
            value: value as u32,
        }
    }

    /// Returns the numeric value of the code point.
    #[inline]
    pub fn to_u32(self) -> u32 {
        self.value
    }

    /// Returns the numeric value of the code point if it is a leading surrogate.
    #[inline]
    pub fn to_lead_surrogate(self) -> Option<LeadSurrogate> {
        match self.value {
            lead @ 0xD800..=0xDBFF => Some(LeadSurrogate(lead as u16)),
            _ => None,
        }
    }

    /// Returns the numeric value of the code point if it is a trailing surrogate.
    #[inline]
    pub fn to_trail_surrogate(self) -> Option<TrailSurrogate> {
        match self.value {
            trail @ 0xDC00..=0xDFFF => Some(TrailSurrogate(trail as u16)),
            _ => None,
        }
    }

    /// Optionally returns a Unicode scalar value for the code point.
    ///
    /// Returns `None` if the code point is a surrogate (from U+D800 to U+DFFF).
    #[inline]
    pub fn to_char(self) -> Option<char> {
        match self.value {
            0xD800..=0xDFFF => None,
            _ => Some(unsafe { char::from_u32_unchecked(self.value) }),
        }
    }

    /// Returns a Unicode scalar value for the code point.
    ///
    /// Returns `'\u{FFFD}'` (the replacement character “�”)
    /// if the code point is a surrogate (from U+D800 to U+DFFF).
    #[inline]
    pub fn to_char_lossy(self) -> char {
        self.to_char().unwrap_or('\u{FFFD}')
    }

    pub fn is_char_and(self, f: impl FnOnce(char) -> bool) -> bool {
        self.to_char().is_some_and(f)
    }

    pub fn encode_wtf8(self, dst: &mut [u8]) -> &mut Wtf8 {
        unsafe { Wtf8::from_mut_bytes_unchecked(encode_utf8_raw(self.value, dst)) }
    }

    pub fn len_wtf8(&self) -> usize {
        len_utf8(self.value)
    }

    pub fn is_ascii(&self) -> bool {
        self.is_char_and(|c| c.is_ascii())
    }
}

impl From<u16> for CodePoint {
    fn from(value: u16) -> Self {
        unsafe { Self::from_u32_unchecked(value.into()) }
    }
}

impl From<u8> for CodePoint {
    fn from(value: u8) -> Self {
        char::from(value).into()
    }
}

impl From<char> for CodePoint {
    fn from(value: char) -> Self {
        Self::from_char(value)
    }
}

impl From<ascii::AsciiChar> for CodePoint {
    fn from(value: ascii::AsciiChar) -> Self {
        Self::from_char(value.into())
    }
}

impl From<CodePoint> for Wtf8Buf {
    fn from(ch: CodePoint) -> Self {
        ch.encode_wtf8(&mut [0; MAX_LEN_UTF8]).to_owned()
    }
}

impl PartialEq<char> for CodePoint {
    fn eq(&self, other: &char) -> bool {
        self.to_u32() == *other as u32
    }
}
impl PartialEq<CodePoint> for char {
    fn eq(&self, other: &CodePoint) -> bool {
        *self as u32 == other.to_u32()
    }
}

#[derive(Clone, Copy)]
pub struct LeadSurrogate(u16);

#[derive(Clone, Copy)]
pub struct TrailSurrogate(u16);

impl LeadSurrogate {
    pub fn merge(self, trail: TrailSurrogate) -> char {
        decode_surrogate_pair(self.0, trail.0)
    }
}

/// An owned, growable string of well-formed WTF-8 data.
///
/// Similar to `String`, but can additionally contain surrogate code points
/// if they’re not in a surrogate pair.
#[derive(Eq, PartialEq, Ord, PartialOrd, Clone, Default)]
pub struct Wtf8Buf {
    bytes: Vec<u8>,
}

impl ops::Deref for Wtf8Buf {
    type Target = Wtf8;

    fn deref(&self) -> &Wtf8 {
        self.as_slice()
    }
}

impl ops::DerefMut for Wtf8Buf {
    fn deref_mut(&mut self) -> &mut Wtf8 {
        self.as_mut_slice()
    }
}

impl Borrow<Wtf8> for Wtf8Buf {
    fn borrow(&self) -> &Wtf8 {
        self
    }
}

/// Formats the string in double quotes, with characters escaped according to
/// [`char::escape_debug`] and unpaired surrogates represented as `\u{xxxx}`,
/// where each `x` is a hexadecimal digit.
///
/// For example, the code units [U+0061, U+D800, U+000A] are formatted as
/// `"a\u{D800}\n"`.
impl fmt::Debug for Wtf8Buf {
    #[inline]
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, formatter)
    }
}

/// Formats the string with unpaired surrogates substituted with the replacement
/// character, U+FFFD.
impl fmt::Display for Wtf8Buf {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&**self, formatter)
    }
}

impl Wtf8Buf {
    /// Creates a new, empty WTF-8 string.
    #[inline]
    pub fn new() -> Wtf8Buf {
        Wtf8Buf::default()
    }

    /// Creates a new, empty WTF-8 string with pre-allocated capacity for `capacity` bytes.
    #[inline]
    pub fn with_capacity(capacity: usize) -> Wtf8Buf {
        Wtf8Buf {
            bytes: Vec::with_capacity(capacity),
        }
    }

    /// Creates a WTF-8 string from a WTF-8 byte vec.
    ///
    /// # Safety
    ///
    /// `value` must contain valid WTF-8.
    #[inline]
    pub unsafe fn from_bytes_unchecked(value: Vec<u8>) -> Wtf8Buf {
        Wtf8Buf { bytes: value }
    }

    /// Create a WTF-8 string from a WTF-8 byte vec.
    pub fn from_bytes(value: Vec<u8>) -> Result<Self, Vec<u8>> {
        match Wtf8::from_bytes(&value) {
            Some(_) => Ok(unsafe { Self::from_bytes_unchecked(value) }),
            None => Err(value),
        }
    }

    /// Creates a WTF-8 string from a UTF-8 `String`.
    ///
    /// This takes ownership of the `String` and does not copy.
    ///
    /// Since WTF-8 is a superset of UTF-8, this always succeeds.
    #[inline]
    pub fn from_string(string: String) -> Wtf8Buf {
        Wtf8Buf {
            bytes: string.into_bytes(),
        }
    }

    pub fn clear(&mut self) {
        self.bytes.clear();
    }

    /// Creates a WTF-8 string from a potentially ill-formed UTF-16 slice of 16-bit code units.
    ///
    /// This is lossless: calling `.encode_wide()` on the resulting string
    /// will always return the original code units.
    pub fn from_wide(v: &[u16]) -> Wtf8Buf {
        let mut string = Wtf8Buf::with_capacity(v.len());
        for item in char::decode_utf16(v.iter().cloned()) {
            match item {
                Ok(ch) => string.push_char(ch),
                Err(surrogate) => {
                    let surrogate = surrogate.unpaired_surrogate();
                    // Surrogates are known to be in the code point range.
                    let code_point = CodePoint::from(surrogate);
                    // Skip the WTF-8 concatenation check,
                    // surrogate pairs are already decoded by decode_utf16
                    string.push(code_point);
                }
            }
        }
        string
    }

    #[inline]
    pub fn as_slice(&self) -> &Wtf8 {
        unsafe { Wtf8::from_bytes_unchecked(&self.bytes) }
    }

    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut Wtf8 {
        // Safety: `Wtf8` doesn't expose any way to mutate the bytes that would
        // cause them to change from well-formed UTF-8 to ill-formed UTF-8,
        // which would break the assumptions of the `is_known_utf8` field.
        unsafe { Wtf8::from_mut_bytes_unchecked(&mut self.bytes) }
    }

    /// Reserves capacity for at least `additional` more bytes to be inserted
    /// in the given `Wtf8Buf`.
    /// The collection may reserve more space to avoid frequent reallocations.
    ///
    /// # Panics
    ///
    /// Panics if the new capacity exceeds `isize::MAX` bytes.
    #[inline]
    pub fn reserve(&mut self, additional: usize) {
        self.bytes.reserve(additional)
    }

    /// Tries to reserve capacity for at least `additional` more bytes to be
    /// inserted in the given `Wtf8Buf`. The `Wtf8Buf` may reserve more space to
    /// avoid frequent reallocations. After calling `try_reserve`, capacity will
    /// be greater than or equal to `self.len() + additional`. Does nothing if
    /// capacity is already sufficient. This method preserves the contents even
    /// if an error occurs.
    ///
    /// # Errors
    ///
    /// If the capacity overflows, or the allocator reports a failure, then an error
    /// is returned.
    #[inline]
    pub fn try_reserve(&mut self, additional: usize) -> Result<(), TryReserveError> {
        self.bytes.try_reserve(additional)
    }

    #[inline]
    pub fn reserve_exact(&mut self, additional: usize) {
        self.bytes.reserve_exact(additional)
    }

    /// Tries to reserve the minimum capacity for exactly `additional` more
    /// bytes to be inserted in the given `Wtf8Buf`. After calling
    /// `try_reserve_exact`, capacity will be greater than or equal to
    /// `self.len() + additional` if it returns `Ok(())`.
    /// Does nothing if the capacity is already sufficient.
    ///
    /// Note that the allocator may give the `Wtf8Buf` more space than it
    /// requests. Therefore, capacity can not be relied upon to be precisely
    /// minimal. Prefer [`try_reserve`] if future insertions are expected.
    ///
    /// [`try_reserve`]: Wtf8Buf::try_reserve
    ///
    /// # Errors
    ///
    /// If the capacity overflows, or the allocator reports a failure, then an error
    /// is returned.
    #[inline]
    pub fn try_reserve_exact(&mut self, additional: usize) -> Result<(), TryReserveError> {
        self.bytes.try_reserve_exact(additional)
    }

    #[inline]
    pub fn shrink_to_fit(&mut self) {
        self.bytes.shrink_to_fit()
    }

    #[inline]
    pub fn shrink_to(&mut self, min_capacity: usize) {
        self.bytes.shrink_to(min_capacity)
    }

    #[inline]
    pub fn leak<'a>(self) -> &'a mut Wtf8 {
        unsafe { Wtf8::from_mut_bytes_unchecked(self.bytes.leak()) }
    }

    /// Returns the number of bytes that this string buffer can hold without reallocating.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.bytes.capacity()
    }

    /// Append a UTF-8 slice at the end of the string.
    #[inline]
    pub fn push_str(&mut self, other: &str) {
        self.bytes.extend_from_slice(other.as_bytes())
    }

    /// Append a WTF-8 slice at the end of the string.
    #[inline]
    pub fn push_wtf8(&mut self, other: &Wtf8) {
        self.bytes.extend_from_slice(&other.bytes);
    }

    /// Append a Unicode scalar value at the end of the string.
    #[inline]
    pub fn push_char(&mut self, c: char) {
        self.push(CodePoint::from_char(c))
    }

    /// Append a code point at the end of the string.
    #[inline]
    pub fn push(&mut self, code_point: CodePoint) {
        self.push_wtf8(code_point.encode_wtf8(&mut [0; MAX_LEN_UTF8]))
    }

    pub fn pop(&mut self) -> Option<CodePoint> {
        let ch = self.code_points().next_back()?;
        let new_len = self.len() - ch.len_wtf8();
        self.bytes.truncate(new_len);
        Some(ch)
    }

    /// Shortens a string to the specified length.
    ///
    /// # Panics
    ///
    /// Panics if `new_len` > current length,
    /// or if `new_len` is not a code point boundary.
    #[inline]
    pub fn truncate(&mut self, new_len: usize) {
        assert!(is_code_point_boundary(self, new_len));
        self.bytes.truncate(new_len)
    }

    /// Inserts a codepoint into this `Wtf8Buf` at a byte position.
    #[inline]
    pub fn insert(&mut self, idx: usize, c: CodePoint) {
        self.insert_wtf8(idx, c.encode_wtf8(&mut [0; MAX_LEN_UTF8]))
    }

    /// Inserts a WTF-8 slice into this `Wtf8Buf` at a byte position.
    #[inline]
    pub fn insert_wtf8(&mut self, idx: usize, w: &Wtf8) {
        assert!(is_code_point_boundary(self, idx));

        self.bytes.insert_str(idx, w)
    }

    /// Consumes the WTF-8 string and tries to convert it to a vec of bytes.
    #[inline]
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// Consumes the WTF-8 string and tries to convert it to UTF-8.
    ///
    /// This does not copy the data.
    ///
    /// If the contents are not well-formed UTF-8
    /// (that is, if the string contains surrogates),
    /// the original WTF-8 string is returned instead.
    pub fn into_string(self) -> Result<String, Wtf8Buf> {
        if self.is_utf8() {
            Ok(unsafe { String::from_utf8_unchecked(self.bytes) })
        } else {
            Err(self)
        }
    }

    /// Consumes the WTF-8 string and converts it lossily to UTF-8.
    ///
    /// This does not copy the data (but may overwrite parts of it in place).
    ///
    /// Surrogates are replaced with `"\u{FFFD}"` (the replacement character “�”)
    pub fn into_string_lossy(mut self) -> String {
        let mut pos = 0;
        while let Some((surrogate_pos, _)) = self.next_surrogate(pos) {
            pos = surrogate_pos + 3;
            // Surrogates and the replacement character are all 3 bytes, so
            // they can substituted in-place.
            self.bytes[surrogate_pos..pos].copy_from_slice(UTF8_REPLACEMENT_CHARACTER.as_bytes());
        }
        unsafe { String::from_utf8_unchecked(self.bytes) }
    }

    /// Converts this `Wtf8Buf` into a boxed `Wtf8`.
    #[inline]
    pub fn into_box(self) -> Box<Wtf8> {
        // SAFETY: relies on `Wtf8` being `repr(transparent)`.
        unsafe { mem::transmute(self.bytes.into_boxed_slice()) }
    }

    /// Converts a `Box<Wtf8>` into a `Wtf8Buf`.
    pub fn from_box(boxed: Box<Wtf8>) -> Wtf8Buf {
        let bytes: Box<[u8]> = unsafe { mem::transmute(boxed) };
        Wtf8Buf {
            bytes: bytes.into_vec(),
        }
    }
}

/// Creates a new WTF-8 string from an iterator of code points.
///
/// This replaces surrogate code point pairs with supplementary code points,
/// like concatenating ill-formed UTF-16 strings effectively would.
impl FromIterator<CodePoint> for Wtf8Buf {
    fn from_iter<T: IntoIterator<Item = CodePoint>>(iter: T) -> Wtf8Buf {
        let mut string = Wtf8Buf::new();
        string.extend(iter);
        string
    }
}

/// Append code points from an iterator to the string.
///
/// This replaces surrogate code point pairs with supplementary code points,
/// like concatenating ill-formed UTF-16 strings effectively would.
impl Extend<CodePoint> for Wtf8Buf {
    fn extend<T: IntoIterator<Item = CodePoint>>(&mut self, iter: T) {
        let iterator = iter.into_iter();
        let (low, _high) = iterator.size_hint();
        // Lower bound of one byte per code point (ASCII only)
        self.bytes.reserve(low);
        iterator.for_each(move |code_point| self.push(code_point));
    }
}

impl Extend<char> for Wtf8Buf {
    fn extend<T: IntoIterator<Item = char>>(&mut self, iter: T) {
        self.extend(iter.into_iter().map(CodePoint::from))
    }
}

impl<W: AsRef<Wtf8>> Extend<W> for Wtf8Buf {
    fn extend<T: IntoIterator<Item = W>>(&mut self, iter: T) {
        iter.into_iter()
            .for_each(move |w| self.push_wtf8(w.as_ref()));
    }
}

impl<W: AsRef<Wtf8>> FromIterator<W> for Wtf8Buf {
    fn from_iter<T: IntoIterator<Item = W>>(iter: T) -> Self {
        let mut buf = Wtf8Buf::new();
        iter.into_iter().for_each(|w| buf.push_wtf8(w.as_ref()));
        buf
    }
}

impl Hash for Wtf8Buf {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Wtf8::hash(self, state)
    }
}

impl AsRef<Wtf8> for Wtf8Buf {
    fn as_ref(&self) -> &Wtf8 {
        self
    }
}

impl From<String> for Wtf8Buf {
    fn from(s: String) -> Self {
        Wtf8Buf::from_string(s)
    }
}

impl From<&str> for Wtf8Buf {
    fn from(s: &str) -> Self {
        Wtf8Buf::from_string(s.to_owned())
    }
}

impl From<ascii::AsciiString> for Wtf8Buf {
    fn from(s: ascii::AsciiString) -> Self {
        Wtf8Buf::from_string(s.into())
    }
}

/// A borrowed slice of well-formed WTF-8 data.
///
/// Similar to `&str`, but can additionally contain surrogate code points
/// if they’re not in a surrogate pair.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
pub struct Wtf8 {
    bytes: [u8],
}

impl AsRef<Wtf8> for Wtf8 {
    fn as_ref(&self) -> &Wtf8 {
        self
    }
}

impl ToOwned for Wtf8 {
    type Owned = Wtf8Buf;
    fn to_owned(&self) -> Self::Owned {
        self.to_wtf8_buf()
    }
    fn clone_into(&self, buf: &mut Self::Owned) {
        self.bytes.clone_into(&mut buf.bytes);
    }
}

impl PartialEq<str> for Wtf8 {
    fn eq(&self, other: &str) -> bool {
        self.as_bytes().eq(other.as_bytes())
    }
}

/// Formats the string in double quotes, with characters escaped according to
/// [`char::escape_debug`] and unpaired surrogates represented as `\u{xxxx}`,
/// where each `x` is a hexadecimal digit.
impl fmt::Debug for Wtf8 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn write_str_escaped(f: &mut fmt::Formatter<'_>, s: &str) -> fmt::Result {
            use std::fmt::Write;
            for c in s.chars().flat_map(|c| c.escape_debug()) {
                f.write_char(c)?
            }
            Ok(())
        }

        formatter.write_str("\"")?;
        let mut pos = 0;
        while let Some((surrogate_pos, surrogate)) = self.next_surrogate(pos) {
            write_str_escaped(formatter, unsafe {
                str::from_utf8_unchecked(&self.bytes[pos..surrogate_pos])
            })?;
            write!(formatter, "\\u{{{:x}}}", surrogate)?;
            pos = surrogate_pos + 3;
        }
        write_str_escaped(formatter, unsafe {
            str::from_utf8_unchecked(&self.bytes[pos..])
        })?;
        formatter.write_str("\"")
    }
}

/// Formats the string with unpaired surrogates substituted with the replacement
/// character, U+FFFD.
impl fmt::Display for Wtf8 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let wtf8_bytes = &self.bytes;
        let mut pos = 0;
        loop {
            match self.next_surrogate(pos) {
                Some((surrogate_pos, _)) => {
                    formatter.write_str(unsafe {
                        str::from_utf8_unchecked(&wtf8_bytes[pos..surrogate_pos])
                    })?;
                    formatter.write_str(UTF8_REPLACEMENT_CHARACTER)?;
                    pos = surrogate_pos + 3;
                }
                None => {
                    let s = unsafe { str::from_utf8_unchecked(&wtf8_bytes[pos..]) };
                    if pos == 0 {
                        return s.fmt(formatter);
                    } else {
                        return formatter.write_str(s);
                    }
                }
            }
        }
    }
}

impl Default for &Wtf8 {
    fn default() -> Self {
        unsafe { Wtf8::from_bytes_unchecked(&[]) }
    }
}

impl Hash for Wtf8 {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write(self.as_bytes());
        state.write_u8(0xff);
    }
}

impl Wtf8 {
    /// Creates a WTF-8 slice from a UTF-8 `&str` slice.
    ///
    /// Since WTF-8 is a superset of UTF-8, this always succeeds.
    #[inline]
    pub fn new<S: AsRef<Wtf8> + ?Sized>(value: &S) -> &Wtf8 {
        value.as_ref()
    }

    /// Creates a WTF-8 slice from a WTF-8 byte slice.
    ///
    /// # Safety
    ///
    /// `value` must contain valid WTF-8.
    #[inline]
    pub unsafe fn from_bytes_unchecked(value: &[u8]) -> &Wtf8 {
        // SAFETY: start with &[u8], end with fancy &[u8]
        unsafe { &*(value as *const [u8] as *const Wtf8) }
    }

    /// Creates a mutable WTF-8 slice from a mutable WTF-8 byte slice.
    ///
    /// Since the byte slice is not checked for valid WTF-8, this functions is
    /// marked unsafe.
    #[inline]
    unsafe fn from_mut_bytes_unchecked(value: &mut [u8]) -> &mut Wtf8 {
        // SAFETY: start with &mut [u8], end with fancy &mut [u8]
        unsafe { &mut *(value as *mut [u8] as *mut Wtf8) }
    }

    /// Create a WTF-8 slice from a WTF-8 byte slice.
    //
    // whooops! using WTF-8 for interchange!
    #[inline]
    pub fn from_bytes(b: &[u8]) -> Option<&Self> {
        let mut rest = b;
        while let Err(e) = std::str::from_utf8(rest) {
            rest = &rest[e.valid_up_to()..];
            let _ = Self::decode_surrogate(rest)?;
            rest = &rest[3..];
        }
        Some(unsafe { Wtf8::from_bytes_unchecked(b) })
    }

    fn decode_surrogate(b: &[u8]) -> Option<CodePoint> {
        let [0xed, b2 @ (0xa0..), b3, ..] = *b else {
            return None;
        };
        Some(decode_surrogate(b2, b3).into())
    }

    /// Returns the length, in WTF-8 bytes.
    #[inline]
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Returns the code point at `position` if it is in the ASCII range,
    /// or `b'\xFF'` otherwise.
    ///
    /// # Panics
    ///
    /// Panics if `position` is beyond the end of the string.
    #[inline]
    pub fn ascii_byte_at(&self, position: usize) -> u8 {
        match self.bytes[position] {
            ascii_byte @ 0x00..=0x7F => ascii_byte,
            _ => 0xFF,
        }
    }

    /// Returns an iterator for the string’s code points.
    #[inline]
    pub fn code_points(&self) -> Wtf8CodePoints<'_> {
        Wtf8CodePoints {
            bytes: self.bytes.iter(),
        }
    }

    /// Returns an iterator for the string’s code points and their indices.
    #[inline]
    pub fn code_point_indices(&self) -> Wtf8CodePointIndices<'_> {
        Wtf8CodePointIndices {
            front_offset: 0,
            iter: self.code_points(),
        }
    }

    /// Access raw bytes of WTF-8 data
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Tries to convert the string to UTF-8 and return a `&str` slice.
    ///
    /// Returns `None` if the string contains surrogates.
    ///
    /// This does not copy the data.
    #[inline]
    pub fn as_str(&self) -> Result<&str, str::Utf8Error> {
        str::from_utf8(&self.bytes)
    }

    /// Creates an owned `Wtf8Buf` from a borrowed `Wtf8`.
    pub fn to_wtf8_buf(&self) -> Wtf8Buf {
        Wtf8Buf {
            bytes: self.bytes.to_vec(),
        }
    }

    /// Lossily converts the string to UTF-8.
    /// Returns a UTF-8 `&str` slice if the contents are well-formed in UTF-8.
    ///
    /// Surrogates are replaced with `"\u{FFFD}"` (the replacement character “�”).
    ///
    /// This only copies the data if necessary (if it contains any surrogate).
    pub fn to_string_lossy(&self) -> Cow<'_, str> {
        let Some((surrogate_pos, _)) = self.next_surrogate(0) else {
            return Cow::Borrowed(unsafe { str::from_utf8_unchecked(&self.bytes) });
        };
        let wtf8_bytes = &self.bytes;
        let mut utf8_bytes = Vec::with_capacity(self.len());
        utf8_bytes.extend_from_slice(&wtf8_bytes[..surrogate_pos]);
        utf8_bytes.extend_from_slice(UTF8_REPLACEMENT_CHARACTER.as_bytes());
        let mut pos = surrogate_pos + 3;
        loop {
            match self.next_surrogate(pos) {
                Some((surrogate_pos, _)) => {
                    utf8_bytes.extend_from_slice(&wtf8_bytes[pos..surrogate_pos]);
                    utf8_bytes.extend_from_slice(UTF8_REPLACEMENT_CHARACTER.as_bytes());
                    pos = surrogate_pos + 3;
                }
                None => {
                    utf8_bytes.extend_from_slice(&wtf8_bytes[pos..]);
                    return Cow::Owned(unsafe { String::from_utf8_unchecked(utf8_bytes) });
                }
            }
        }
    }

    /// Converts the WTF-8 string to potentially ill-formed UTF-16
    /// and return an iterator of 16-bit code units.
    ///
    /// This is lossless:
    /// calling `Wtf8Buf::from_ill_formed_utf16` on the resulting code units
    /// would always return the original WTF-8 string.
    #[inline]
    pub fn encode_wide(&self) -> EncodeWide<'_> {
        EncodeWide {
            code_points: self.code_points(),
            extra: 0,
        }
    }

    pub fn chunks(&self) -> Wtf8Chunks<'_> {
        Wtf8Chunks { wtf8: self }
    }

    pub fn map_utf8<'a, I>(&'a self, f: impl Fn(&'a str) -> I) -> impl Iterator<Item = CodePoint>
    where
        I: Iterator<Item = char>,
    {
        self.chunks().flat_map(move |chunk| match chunk {
            Wtf8Chunk::Utf8(s) => Either::Left(f(s).map_into()),
            Wtf8Chunk::Surrogate(c) => Either::Right(std::iter::once(c)),
        })
    }

    #[inline]
    fn next_surrogate(&self, mut pos: usize) -> Option<(usize, u16)> {
        let mut iter = self.bytes[pos..].iter();
        loop {
            let b = *iter.next()?;
            if b < 0x80 {
                pos += 1;
            } else if b < 0xE0 {
                iter.next();
                pos += 2;
            } else if b == 0xED {
                match (iter.next(), iter.next()) {
                    (Some(&b2), Some(&b3)) if b2 >= 0xA0 => {
                        return Some((pos, decode_surrogate(b2, b3)));
                    }
                    _ => pos += 3,
                }
            } else if b < 0xF0 {
                iter.next();
                iter.next();
                pos += 3;
            } else {
                iter.next();
                iter.next();
                iter.next();
                pos += 4;
            }
        }
    }

    pub fn is_code_point_boundary(&self, index: usize) -> bool {
        is_code_point_boundary(self, index)
    }

    /// Boxes this `Wtf8`.
    #[inline]
    pub fn into_box(&self) -> Box<Wtf8> {
        let boxed: Box<[u8]> = self.bytes.into();
        unsafe { mem::transmute(boxed) }
    }

    /// Creates a boxed, empty `Wtf8`.
    pub fn empty_box() -> Box<Wtf8> {
        let boxed: Box<[u8]> = Default::default();
        unsafe { mem::transmute(boxed) }
    }

    #[inline]
    pub fn make_ascii_lowercase(&mut self) {
        self.bytes.make_ascii_lowercase()
    }

    #[inline]
    pub fn make_ascii_uppercase(&mut self) {
        self.bytes.make_ascii_uppercase()
    }

    #[inline]
    pub fn to_ascii_lowercase(&self) -> Wtf8Buf {
        Wtf8Buf {
            bytes: self.bytes.to_ascii_lowercase(),
        }
    }

    #[inline]
    pub fn to_ascii_uppercase(&self) -> Wtf8Buf {
        Wtf8Buf {
            bytes: self.bytes.to_ascii_uppercase(),
        }
    }

    #[inline]
    pub fn is_ascii(&self) -> bool {
        self.bytes.is_ascii()
    }

    #[inline]
    pub fn is_utf8(&self) -> bool {
        self.next_surrogate(0).is_none()
    }

    #[inline]
    pub fn eq_ignore_ascii_case(&self, other: &Self) -> bool {
        self.bytes.eq_ignore_ascii_case(&other.bytes)
    }

    pub fn split(&self, pat: &Wtf8) -> impl Iterator<Item = &Self> {
        self.as_bytes()
            .split_str(pat)
            .map(|w| unsafe { Wtf8::from_bytes_unchecked(w) })
    }

    pub fn splitn(&self, n: usize, pat: &Wtf8) -> impl Iterator<Item = &Self> {
        self.as_bytes()
            .splitn_str(n, pat)
            .map(|w| unsafe { Wtf8::from_bytes_unchecked(w) })
    }

    pub fn rsplit(&self, pat: &Wtf8) -> impl Iterator<Item = &Self> {
        self.as_bytes()
            .rsplit_str(pat)
            .map(|w| unsafe { Wtf8::from_bytes_unchecked(w) })
    }

    pub fn rsplitn(&self, n: usize, pat: &Wtf8) -> impl Iterator<Item = &Self> {
        self.as_bytes()
            .rsplitn_str(n, pat)
            .map(|w| unsafe { Wtf8::from_bytes_unchecked(w) })
    }

    pub fn trim(&self) -> &Self {
        let w = self.bytes.trim();
        unsafe { Wtf8::from_bytes_unchecked(w) }
    }

    pub fn trim_start(&self) -> &Self {
        let w = self.bytes.trim_start();
        unsafe { Wtf8::from_bytes_unchecked(w) }
    }

    pub fn trim_end(&self) -> &Self {
        let w = self.bytes.trim_end();
        unsafe { Wtf8::from_bytes_unchecked(w) }
    }

    pub fn trim_start_matches(&self, f: impl Fn(CodePoint) -> bool) -> &Self {
        let mut iter = self.code_points();
        loop {
            let old = iter.clone();
            match iter.next().map(&f) {
                Some(true) => continue,
                Some(false) => {
                    iter = old;
                    break;
                }
                None => return iter.as_wtf8(),
            }
        }
        iter.as_wtf8()
    }

    pub fn trim_end_matches(&self, f: impl Fn(CodePoint) -> bool) -> &Self {
        let mut iter = self.code_points();
        loop {
            let old = iter.clone();
            match iter.next_back().map(&f) {
                Some(true) => continue,
                Some(false) => {
                    iter = old;
                    break;
                }
                None => return iter.as_wtf8(),
            }
        }
        iter.as_wtf8()
    }

    pub fn trim_matches(&self, f: impl Fn(CodePoint) -> bool) -> &Self {
        self.trim_start_matches(&f).trim_end_matches(&f)
    }

    pub fn find(&self, pat: &Wtf8) -> Option<usize> {
        memchr::memmem::find(self.as_bytes(), pat.as_bytes())
    }

    pub fn rfind(&self, pat: &Wtf8) -> Option<usize> {
        memchr::memmem::rfind(self.as_bytes(), pat.as_bytes())
    }

    pub fn find_iter(&self, pat: &Wtf8) -> impl Iterator<Item = usize> {
        memchr::memmem::find_iter(self.as_bytes(), pat.as_bytes())
    }

    pub fn rfind_iter(&self, pat: &Wtf8) -> impl Iterator<Item = usize> {
        memchr::memmem::rfind_iter(self.as_bytes(), pat.as_bytes())
    }

    pub fn contains(&self, pat: &Wtf8) -> bool {
        self.bytes.contains_str(pat)
    }

    pub fn contains_code_point(&self, pat: CodePoint) -> bool {
        self.bytes
            .contains_str(pat.encode_wtf8(&mut [0; MAX_LEN_UTF8]))
    }

    pub fn get(&self, range: impl ops::RangeBounds<usize>) -> Option<&Self> {
        let start = match range.start_bound() {
            ops::Bound::Included(&i) => i,
            ops::Bound::Excluded(&i) => i.saturating_add(1),
            ops::Bound::Unbounded => 0,
        };
        let end = match range.end_bound() {
            ops::Bound::Included(&i) => i.saturating_add(1),
            ops::Bound::Excluded(&i) => i,
            ops::Bound::Unbounded => self.len(),
        };
        // is_code_point_boundary checks that the index is in [0, .len()]
        if start <= end && is_code_point_boundary(self, start) && is_code_point_boundary(self, end)
        {
            Some(unsafe { slice_unchecked(self, start, end) })
        } else {
            None
        }
    }

    pub fn ends_with(&self, w: &Wtf8) -> bool {
        self.bytes.ends_with_str(w)
    }

    pub fn starts_with(&self, w: &Wtf8) -> bool {
        self.bytes.starts_with_str(w)
    }

    pub fn strip_prefix(&self, w: &Wtf8) -> Option<&Self> {
        self.bytes
            .strip_prefix(w.as_bytes())
            .map(|w| unsafe { Wtf8::from_bytes_unchecked(w) })
    }

    pub fn strip_suffix(&self, w: &Wtf8) -> Option<&Self> {
        self.bytes
            .strip_suffix(w.as_bytes())
            .map(|w| unsafe { Wtf8::from_bytes_unchecked(w) })
    }

    pub fn replace(&self, from: &Wtf8, to: &Wtf8) -> Wtf8Buf {
        let w = self.bytes.replace(from, to);
        unsafe { Wtf8Buf::from_bytes_unchecked(w) }
    }

    pub fn replacen(&self, from: &Wtf8, to: &Wtf8, n: usize) -> Wtf8Buf {
        let w = self.bytes.replacen(from, to, n);
        unsafe { Wtf8Buf::from_bytes_unchecked(w) }
    }
}

impl AsRef<Wtf8> for str {
    fn as_ref(&self) -> &Wtf8 {
        unsafe { Wtf8::from_bytes_unchecked(self.as_bytes()) }
    }
}

impl AsRef<[u8]> for Wtf8 {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

/// Returns a slice of the given string for the byte range \[`begin`..`end`).
///
/// # Panics
///
/// Panics when `begin` and `end` do not point to code point boundaries,
/// or point beyond the end of the string.
impl ops::Index<ops::Range<usize>> for Wtf8 {
    type Output = Wtf8;

    #[inline]
    #[track_caller]
    fn index(&self, range: ops::Range<usize>) -> &Wtf8 {
        // is_code_point_boundary checks that the index is in [0, .len()]
        if range.start <= range.end
            && is_code_point_boundary(self, range.start)
            && is_code_point_boundary(self, range.end)
        {
            unsafe { slice_unchecked(self, range.start, range.end) }
        } else {
            slice_error_fail(self, range.start, range.end)
        }
    }
}

/// Returns a slice of the given string from byte `begin` to its end.
///
/// # Panics
///
/// Panics when `begin` is not at a code point boundary,
/// or is beyond the end of the string.
impl ops::Index<ops::RangeFrom<usize>> for Wtf8 {
    type Output = Wtf8;

    #[inline]
    #[track_caller]
    fn index(&self, range: ops::RangeFrom<usize>) -> &Wtf8 {
        // is_code_point_boundary checks that the index is in [0, .len()]
        if is_code_point_boundary(self, range.start) {
            unsafe { slice_unchecked(self, range.start, self.len()) }
        } else {
            slice_error_fail(self, range.start, self.len())
        }
    }
}

/// Returns a slice of the given string from its beginning to byte `end`.
///
/// # Panics
///
/// Panics when `end` is not at a code point boundary,
/// or is beyond the end of the string.
impl ops::Index<ops::RangeTo<usize>> for Wtf8 {
    type Output = Wtf8;

    #[inline]
    #[track_caller]
    fn index(&self, range: ops::RangeTo<usize>) -> &Wtf8 {
        // is_code_point_boundary checks that the index is in [0, .len()]
        if is_code_point_boundary(self, range.end) {
            unsafe { slice_unchecked(self, 0, range.end) }
        } else {
            slice_error_fail(self, 0, range.end)
        }
    }
}

impl ops::Index<ops::RangeFull> for Wtf8 {
    type Output = Wtf8;

    #[inline]
    fn index(&self, _range: ops::RangeFull) -> &Wtf8 {
        self
    }
}

#[inline]
fn decode_surrogate(second_byte: u8, third_byte: u8) -> u16 {
    // The first byte is assumed to be 0xED
    0xD800 | (second_byte as u16 & 0x3F) << 6 | third_byte as u16 & 0x3F
}

#[inline]
fn decode_surrogate_pair(lead: u16, trail: u16) -> char {
    let code_point = 0x10000 + ((((lead - 0xD800) as u32) << 10) | (trail - 0xDC00) as u32);
    unsafe { char::from_u32_unchecked(code_point) }
}

/// Copied from str::is_char_boundary
#[inline]
fn is_code_point_boundary(slice: &Wtf8, index: usize) -> bool {
    if index == 0 {
        return true;
    }
    match slice.bytes.get(index) {
        None => index == slice.len(),
        Some(&b) => (b as i8) >= -0x40,
    }
}

/// Verify that `index` is at the edge of either a valid UTF-8 codepoint
/// (i.e. a codepoint that's not a surrogate) or of the whole string.
///
/// These are the cases currently permitted by `OsStr::slice_encoded_bytes`.
/// Splitting between surrogates is valid as far as WTF-8 is concerned, but
/// we do not permit it in the public API because WTF-8 is considered an
/// implementation detail.
#[track_caller]
#[inline]
pub fn check_utf8_boundary(slice: &Wtf8, index: usize) {
    if index == 0 {
        return;
    }
    match slice.bytes.get(index) {
        Some(0xED) => (), // Might be a surrogate
        Some(&b) if (b as i8) >= -0x40 => return,
        Some(_) => panic!("byte index {index} is not a codepoint boundary"),
        None if index == slice.len() => return,
        None => panic!("byte index {index} is out of bounds"),
    }
    if slice.bytes[index + 1] >= 0xA0 {
        // There's a surrogate after index. Now check before index.
        if index >= 3 && slice.bytes[index - 3] == 0xED && slice.bytes[index - 2] >= 0xA0 {
            panic!("byte index {index} lies between surrogate codepoints");
        }
    }
}

/// Copied from core::str::raw::slice_unchecked
///
/// # Safety
///
/// `begin` and `end` must be within bounds and on codepoint boundaries.
#[inline]
pub unsafe fn slice_unchecked(s: &Wtf8, begin: usize, end: usize) -> &Wtf8 {
    // SAFETY: memory layout of a &[u8] and &Wtf8 are the same
    unsafe {
        let len = end - begin;
        let start = s.as_bytes().as_ptr().add(begin);
        Wtf8::from_bytes_unchecked(slice::from_raw_parts(start, len))
    }
}

/// Copied from core::str::raw::slice_error_fail
#[inline(never)]
#[track_caller]
pub fn slice_error_fail(s: &Wtf8, begin: usize, end: usize) -> ! {
    assert!(begin <= end);
    panic!("index {begin} and/or {end} in `{s:?}` do not lie on character boundary");
}

/// Iterator for the code points of a WTF-8 string.
///
/// Created with the method `.code_points()`.
#[derive(Clone)]
pub struct Wtf8CodePoints<'a> {
    bytes: slice::Iter<'a, u8>,
}

impl Iterator for Wtf8CodePoints<'_> {
    type Item = CodePoint;

    #[inline]
    fn next(&mut self) -> Option<CodePoint> {
        // SAFETY: `self.bytes` has been created from a WTF-8 string
        unsafe { next_code_point(&mut self.bytes).map(|c| CodePoint { value: c }) }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.bytes.len();
        (len.saturating_add(3) / 4, Some(len))
    }

    fn last(mut self) -> Option<Self::Item> {
        self.next_back()
    }

    fn count(self) -> usize {
        core_str_count::count_chars(self.as_wtf8())
    }
}

impl DoubleEndedIterator for Wtf8CodePoints<'_> {
    #[inline]
    fn next_back(&mut self) -> Option<CodePoint> {
        // SAFETY: `str` invariant says `self.iter` is a valid WTF-8 string and
        // the resulting `ch` is a valid Unicode Code Point.
        unsafe {
            next_code_point_reverse(&mut self.bytes).map(|ch| CodePoint::from_u32_unchecked(ch))
        }
    }
}

impl<'a> Wtf8CodePoints<'a> {
    pub fn as_wtf8(&self) -> &'a Wtf8 {
        unsafe { Wtf8::from_bytes_unchecked(self.bytes.as_slice()) }
    }
}

#[derive(Clone)]
pub struct Wtf8CodePointIndices<'a> {
    front_offset: usize,
    iter: Wtf8CodePoints<'a>,
}

impl Iterator for Wtf8CodePointIndices<'_> {
    type Item = (usize, CodePoint);

    #[inline]
    fn next(&mut self) -> Option<(usize, CodePoint)> {
        let pre_len = self.iter.bytes.len();
        match self.iter.next() {
            None => None,
            Some(ch) => {
                let index = self.front_offset;
                let len = self.iter.bytes.len();
                self.front_offset += pre_len - len;
                Some((index, ch))
            }
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }

    #[inline]
    fn last(mut self) -> Option<(usize, CodePoint)> {
        // No need to go through the entire string.
        self.next_back()
    }

    #[inline]
    fn count(self) -> usize {
        self.iter.count()
    }
}

impl DoubleEndedIterator for Wtf8CodePointIndices<'_> {
    #[inline]
    fn next_back(&mut self) -> Option<(usize, CodePoint)> {
        self.iter.next_back().map(|ch| {
            let index = self.front_offset + self.iter.bytes.len();
            (index, ch)
        })
    }
}

impl FusedIterator for Wtf8CodePointIndices<'_> {}

/// Generates a wide character sequence for potentially ill-formed UTF-16.
#[derive(Clone)]
pub struct EncodeWide<'a> {
    code_points: Wtf8CodePoints<'a>,
    extra: u16,
}

// Copied from libunicode/u_str.rs
impl Iterator for EncodeWide<'_> {
    type Item = u16;

    #[inline]
    fn next(&mut self) -> Option<u16> {
        if self.extra != 0 {
            let tmp = self.extra;
            self.extra = 0;
            return Some(tmp);
        }

        let mut buf = [0; MAX_LEN_UTF16];
        self.code_points.next().map(|code_point| {
            let n = encode_utf16_raw(code_point.value, &mut buf).len();
            if n == 2 {
                self.extra = buf[1];
            }
            buf[0]
        })
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let (low, high) = self.code_points.size_hint();
        let ext = (self.extra != 0) as usize;
        // every code point gets either one u16 or two u16,
        // so this iterator is between 1 or 2 times as
        // long as the underlying iterator.
        (
            low + ext,
            high.and_then(|n| n.checked_mul(2))
                .and_then(|n| n.checked_add(ext)),
        )
    }
}

impl FusedIterator for EncodeWide<'_> {}

pub struct Wtf8Chunks<'a> {
    wtf8: &'a Wtf8,
}

impl<'a> Iterator for Wtf8Chunks<'a> {
    type Item = Wtf8Chunk<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.wtf8.next_surrogate(0) {
            Some((0, surrogate)) => {
                self.wtf8 = &self.wtf8[3..];
                Some(Wtf8Chunk::Surrogate(surrogate.into()))
            }
            Some((n, _)) => {
                let s = unsafe { str::from_utf8_unchecked(&self.wtf8.as_bytes()[..n]) };
                self.wtf8 = &self.wtf8[n..];
                Some(Wtf8Chunk::Utf8(s))
            }
            None => {
                let s =
                    unsafe { str::from_utf8_unchecked(std::mem::take(&mut self.wtf8).as_bytes()) };
                (!s.is_empty()).then_some(Wtf8Chunk::Utf8(s))
            }
        }
    }
}

pub enum Wtf8Chunk<'a> {
    Utf8(&'a str),
    Surrogate(CodePoint),
}

impl Hash for CodePoint {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.value.hash(state)
    }
}

// == BOX IMPLS ==

/// # Safety
///
/// `value` must be valid WTF-8.
pub unsafe fn from_boxed_wtf8_unchecked(value: Box<[u8]>) -> Box<Wtf8> {
    unsafe { Box::from_raw(Box::into_raw(value) as *mut Wtf8) }
}

impl Clone for Box<Wtf8> {
    fn clone(&self) -> Self {
        (&**self).into()
    }
}

impl Default for Box<Wtf8> {
    fn default() -> Self {
        unsafe { from_boxed_wtf8_unchecked(Box::default()) }
    }
}

impl From<&Wtf8> for Box<Wtf8> {
    fn from(w: &Wtf8) -> Self {
        w.into_box()
    }
}

impl From<&str> for Box<Wtf8> {
    fn from(s: &str) -> Self {
        Box::<str>::from(s).into()
    }
}

impl From<Box<str>> for Box<Wtf8> {
    fn from(s: Box<str>) -> Self {
        unsafe { from_boxed_wtf8_unchecked(s.into_boxed_bytes()) }
    }
}

impl From<Box<ascii::AsciiStr>> for Box<Wtf8> {
    fn from(s: Box<ascii::AsciiStr>) -> Self {
        <Box<str>>::from(s).into()
    }
}

impl From<Box<Wtf8>> for Box<[u8]> {
    fn from(w: Box<Wtf8>) -> Self {
        unsafe { Box::from_raw(Box::into_raw(w) as *mut [u8]) }
    }
}

impl From<Wtf8Buf> for Box<Wtf8> {
    fn from(w: Wtf8Buf) -> Self {
        w.into_box()
    }
}

impl From<Box<Wtf8>> for Wtf8Buf {
    fn from(w: Box<Wtf8>) -> Self {
        Wtf8Buf::from_box(w)
    }
}

impl From<String> for Box<Wtf8> {
    fn from(s: String) -> Self {
        s.into_boxed_str().into()
    }
}
