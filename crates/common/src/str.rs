// spell-checker:ignore uncomputed
use crate::atomic::{OncePtr, PyAtomic, Radium};
use crate::format::CharLen;
use crate::wtf8::{CodePoint, Wtf8, Wtf8Buf};
use crate::wtf8_index::Wtf8Index;
use alloc::borrow::Cow;
use ascii::{AsciiChar, AsciiStr, AsciiString};
use core::fmt;
use core::ops::{Bound, RangeBounds};
use core::sync::atomic::Ordering::Relaxed;

#[allow(non_camel_case_types)]
pub type wchar_t = cfg_select! {
    target_arch = "wasm32" => u32,
    _ => libc::wchar_t,
};

/// Utf8 + state.ascii (+ PyUnicode_Kind in future)
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum StrKind {
    Ascii,
    Utf8,
    Wtf8,
}

impl core::ops::BitOr for StrKind {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        match (self, other) {
            (Self::Wtf8, _) | (_, Self::Wtf8) => Self::Wtf8,
            (Self::Utf8, _) | (_, Self::Utf8) => Self::Utf8,
            (Self::Ascii, Self::Ascii) => Self::Ascii,
        }
    }
}

impl StrKind {
    #[must_use]
    pub const fn is_ascii(&self) -> bool {
        matches!(self, Self::Ascii)
    }

    #[must_use]
    pub const fn is_utf8(&self) -> bool {
        matches!(self, Self::Ascii | Self::Utf8)
    }

    #[inline(always)]
    #[must_use]
    pub fn can_encode(&self, code: CodePoint) -> bool {
        match self {
            Self::Ascii => code.is_ascii(),
            Self::Utf8 => code.to_char().is_some(),
            Self::Wtf8 => true,
        }
    }
}

pub trait DeduceStrKind {
    fn str_kind(&self) -> StrKind;
}

impl DeduceStrKind for str {
    fn str_kind(&self) -> StrKind {
        if self.is_ascii() {
            StrKind::Ascii
        } else {
            StrKind::Utf8
        }
    }
}

impl DeduceStrKind for Wtf8 {
    fn str_kind(&self) -> StrKind {
        if self.is_ascii() {
            StrKind::Ascii
        } else if self.is_utf8() {
            StrKind::Utf8
        } else {
            StrKind::Wtf8
        }
    }
}

impl DeduceStrKind for String {
    fn str_kind(&self) -> StrKind {
        (**self).str_kind()
    }
}

impl DeduceStrKind for Wtf8Buf {
    fn str_kind(&self) -> StrKind {
        (**self).str_kind()
    }
}

impl<T: DeduceStrKind + ?Sized> DeduceStrKind for &T {
    fn str_kind(&self) -> StrKind {
        (**self).str_kind()
    }
}

impl<T: DeduceStrKind + ?Sized> DeduceStrKind for Box<T> {
    fn str_kind(&self) -> StrKind {
        (**self).str_kind()
    }
}

#[derive(Debug)]
pub enum PyKindStr<'a> {
    Ascii(&'a AsciiStr),
    Utf8(&'a str),
    Wtf8(&'a Wtf8),
}

/// How far from an end an index is resolved by walking rather than by building
/// the code point index.
///
/// PyPy spells this `MAX_UNROLL_NEXT_CODEPOINT_POS`, in a guard that also asks
/// the JIT whether the index is a constant, so that the walk unrolls. There is
/// no JIT here to ask, and the walk is short rather than free -- but four steps
/// still beat a pass over the whole buffer, and skipping the build is what
/// keeps `s[0]` and `s[1:-1]` on a long string from paying for a table.
const MAX_WALK_TO_INDEX: usize = 4;

#[derive(Debug, Clone)]
pub struct StrData {
    data: Wtf8Buf,
    metadata: StrMetadata,
    index: Wtf8IndexSlot,
}

/// A [`Wtf8Index`] built on first use.
///
/// The table is a pure function of `data`, so publishing it races benignly: a
/// thread that loses the exchange drops its own copy and reads the winner's.
#[derive(Default)]
struct Wtf8IndexSlot(OncePtr<Wtf8Index>);

impl Wtf8IndexSlot {
    #[inline(always)]
    fn new() -> Self {
        Self(OncePtr::new())
    }

    #[inline]
    fn get_or_build(&self, data: &Wtf8, char_len: usize) -> &Wtf8Index {
        let index = self
            .0
            .get_or_init(|| Box::new(Wtf8Index::new(data, char_len)));
        // The slot owns the table, never replaces it, and outlives the borrow.
        unsafe { index.as_ref() }
    }
}

impl fmt::Debug for Wtf8IndexSlot {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.0.get() {
            Some(_) => f.write_str("<built>"),
            None => f.write_str("<unbuilt>"),
        }
    }
}

impl Clone for Wtf8IndexSlot {
    /// A fresh slot: the clone copies the buffer, so it has to index that copy,
    /// and the table is rebuilt on demand rather than eagerly here.
    fn clone(&self) -> Self {
        Self::new()
    }
}

impl Drop for Wtf8IndexSlot {
    fn drop(&mut self) {
        if let Some(index) = self.0.get() {
            drop(unsafe { Box::from_raw(index.as_ptr()) });
        }
    }
}

const STR_KIND_BITS: u32 = 2;
const STR_KIND_SHIFT: u32 = usize::BITS - STR_KIND_BITS;
const STR_LEN_MASK: usize = (1usize << STR_KIND_SHIFT) - 1;
const STR_LEN_UNCOMPUTED: usize = STR_LEN_MASK;

struct StrMetadata(PyAtomic<usize>);

impl fmt::Debug for StrMetadata {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("StrMetadata")
            .field("kind", &self.kind())
            .field("len", &self.cached_len())
            .finish()
    }
}

impl StrMetadata {
    #[inline(always)]
    fn new(kind: StrKind, len: Option<usize>) -> Self {
        Self(Radium::new(Self::encode(kind, len)))
    }

    #[inline(always)]
    fn encode(kind: StrKind, len: Option<usize>) -> usize {
        let kind: usize = match kind {
            StrKind::Ascii => 0,
            StrKind::Utf8 => 1,
            StrKind::Wtf8 => 2,
        };
        let len = len
            .filter(|&len| len < STR_LEN_UNCOMPUTED)
            .unwrap_or(STR_LEN_UNCOMPUTED);
        (kind << STR_KIND_SHIFT) | len
    }

    #[inline(always)]
    fn kind(&self) -> StrKind {
        match self.0.load(Relaxed) >> STR_KIND_SHIFT {
            0 => StrKind::Ascii,
            1 => StrKind::Utf8,
            2 => StrKind::Wtf8,
            _ => unreachable!(),
        }
    }

    #[inline(always)]
    fn cached_len(&self) -> Option<usize> {
        let len = self.0.load(Relaxed) & STR_LEN_MASK;
        (len != STR_LEN_UNCOMPUTED).then_some(len)
    }

    #[inline(always)]
    fn store(&self, kind: StrKind, len: Option<usize>) {
        self.0.store(Self::encode(kind, len), Relaxed);
    }
}

impl Clone for StrMetadata {
    fn clone(&self) -> Self {
        Self(Radium::new(self.0.load(Relaxed)))
    }
}

impl Default for StrData {
    fn default() -> Self {
        Self {
            data: Wtf8Buf::new(),
            metadata: StrMetadata::new(StrKind::Ascii, Some(0)),
            index: Wtf8IndexSlot::new(),
        }
    }
}

impl From<Box<Wtf8>> for StrData {
    fn from(value: Box<Wtf8>) -> Self {
        // doing the check is ~10x faster for ascii, and is actually only 2% slower worst case for
        // non-ascii; see https://github.com/RustPython/RustPython/pull/2586#issuecomment-844611532
        let kind = value.str_kind();
        unsafe { Self::new_str_unchecked(value, kind) }
    }
}

impl From<Box<str>> for StrData {
    #[inline]
    fn from(value: Box<str>) -> Self {
        // doing the check is ~10x faster for ascii, and is actually only 2% slower worst case for
        // non-ascii; see https://github.com/RustPython/RustPython/pull/2586#issuecomment-844611532
        let kind = value.str_kind();
        unsafe { Self::new_str_unchecked(value.into(), kind) }
    }
}

impl From<Box<AsciiStr>> for StrData {
    #[inline]
    fn from(value: Box<AsciiStr>) -> Self {
        Self {
            metadata: StrMetadata::new(StrKind::Ascii, Some(value.len())),
            data: Wtf8Buf::from_box(value.into()),
            index: Wtf8IndexSlot::new(),
        }
    }
}

impl From<AsciiChar> for StrData {
    fn from(ch: AsciiChar) -> Self {
        AsciiString::from(ch).into_boxed_ascii_str().into()
    }
}

impl From<char> for StrData {
    fn from(ch: char) -> Self {
        if let Ok(ch) = ascii::AsciiChar::from_ascii(ch) {
            ch.into()
        } else {
            Self {
                data: Wtf8Buf::from_string(ch.to_string()),
                metadata: StrMetadata::new(StrKind::Utf8, Some(1)),
                index: Wtf8IndexSlot::new(),
            }
        }
    }
}

impl From<CodePoint> for StrData {
    fn from(ch: CodePoint) -> Self {
        if let Some(ch) = ch.to_char() {
            ch.into()
        } else {
            Self {
                data: Wtf8Buf::from(ch),
                metadata: StrMetadata::new(StrKind::Wtf8, Some(1)),
                index: Wtf8IndexSlot::new(),
            }
        }
    }
}

impl StrData {
    /// # Safety
    ///
    /// Given `bytes` must be valid data for given `kind`
    #[must_use]
    pub unsafe fn new_str_unchecked(data: Box<Wtf8>, kind: StrKind) -> Self {
        let len = match kind {
            StrKind::Ascii => Some(data.len()),
            _ => None,
        };
        Self {
            data: Wtf8Buf::from_box(data),
            metadata: StrMetadata::new(kind, len),
            index: Wtf8IndexSlot::new(),
        }
    }

    /// # Safety
    ///
    /// `char_len` must be accurate.
    #[must_use]
    pub unsafe fn new_with_char_len(data: Box<Wtf8>, kind: StrKind, char_len: usize) -> Self {
        Self {
            data: Wtf8Buf::from_box(data),
            metadata: StrMetadata::new(kind, Some(char_len)),
            index: Wtf8IndexSlot::new(),
        }
    }

    #[inline]
    pub fn as_wtf8(&self) -> &Wtf8 {
        self.data.as_slice()
    }

    // TODO: rename to to_str
    #[inline]
    pub fn as_str(&self) -> Option<&str> {
        self.kind()
            .is_utf8()
            .then(|| unsafe { core::str::from_utf8_unchecked(self.data.as_bytes()) })
    }

    pub fn as_ascii(&self) -> Option<&AsciiStr> {
        self.kind()
            .is_ascii()
            .then(|| unsafe { AsciiStr::from_ascii_unchecked(self.data.as_bytes()) })
    }

    pub fn kind(&self) -> StrKind {
        self.metadata.kind()
    }

    #[inline]
    pub fn as_str_kind(&self) -> PyKindStr<'_> {
        match self.kind() {
            StrKind::Ascii => {
                PyKindStr::Ascii(unsafe { AsciiStr::from_ascii_unchecked(self.data.as_bytes()) })
            }
            StrKind::Utf8 => {
                PyKindStr::Utf8(unsafe { core::str::from_utf8_unchecked(self.data.as_bytes()) })
            }
            StrKind::Wtf8 => PyKindStr::Wtf8(&self.data),
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    #[inline]
    pub fn char_len(&self) -> usize {
        self.metadata
            .cached_len()
            .unwrap_or_else(|| self._compute_char_len())
    }

    #[cold]
    fn _compute_char_len(&self) -> usize {
        let len = if let Some(s) = self.as_str() {
            // utf8 chars().count() is optimized
            s.chars().count()
        } else {
            self.data.code_points().count()
        };
        self.metadata.store(self.kind(), Some(len));
        len
    }

    pub fn append(&mut self, other: &Wtf8) {
        if other.is_empty() {
            return;
        }
        let other_kind = other.str_kind();
        let kind = self.kind() | other_kind;
        let char_len = self.metadata.cached_len().and_then(|len| {
            let other_len = match other_kind {
                StrKind::Ascii => other.len(),
                StrKind::Utf8 => unsafe { core::str::from_utf8_unchecked(other.as_bytes()) }
                    .chars()
                    .count(),
                StrKind::Wtf8 => other.code_points().count(),
            };
            len.checked_add(other_len)
        });
        self.data.reserve(other.len());
        self.data.push_wtf8(other);
        self.metadata.store(kind, char_len);
        self.index = Wtf8IndexSlot::new();
    }

    /// The byte offset the `index`-th code point starts at.
    ///
    /// An `index` at or past the end answers the buffer's byte length, so a
    /// caller walking to a bound does not have to special-case it.
    ///
    /// O(1), but the first call on a non-ASCII string builds an index over the
    /// whole buffer, so a caller that resolves a single index and stops is
    /// better served by [`Self::nth_char`].
    pub fn char_index_to_byte(&self, index: usize) -> usize {
        // For ASCII the two units coincide, and the table would be a Nth entry
        // saying N.
        if self.kind().is_ascii() {
            return index.min(self.data.len());
        }
        let char_len = self.char_len();
        if index >= char_len {
            return self.data.len();
        }
        self.index
            .get_or_build(&self.data, char_len)
            .byte_offset(&self.data, index)
    }

    /// The byte offset of code point `index`, for a caller that resolves one
    /// index and stops.
    ///
    /// Building the table costs a pass over the whole buffer, so it is worth it
    /// only for a caller that comes back; an index within
    /// [`MAX_WALK_TO_INDEX`] steps of either end is cheaper to walk to, and
    /// walking keeps `s[0]` on a long string from paying for a table it will
    /// never use again. Anything further in builds, on the reasoning that a
    /// string indexed once in the middle tends to be indexed again.
    fn char_index_to_byte_once(&self, index: usize) -> usize {
        if index <= MAX_WALK_TO_INDEX {
            return self
                .data
                .code_point_indices()
                .nth(index)
                .map_or(self.data.len(), |(byte, _)| byte);
        }
        let from_end = self.char_len() - index;
        if from_end <= MAX_WALK_TO_INDEX {
            return self
                .data
                .code_point_indices()
                .nth_back(from_end - 1)
                .map_or(self.data.len(), |(byte, _)| byte);
        }
        self.char_index_to_byte(index)
    }

    /// The byte range spanned by the code points in `range`, whose end must not
    /// exceed the string's code point count.
    ///
    /// A range that reaches within [`MAX_WALK_TO_INDEX`] of *both* ends is
    /// walked to for the same reason a single index near one end is -- a slice
    /// like `s[1:-1]` should not build a table over the whole string.
    #[must_use]
    pub fn char_range_to_bytes(&self, range: core::ops::Range<usize>) -> core::ops::Range<usize> {
        if self.kind().is_ascii() {
            return range;
        }
        let from_end = self.char_len() - range.end;
        if range.start <= MAX_WALK_TO_INDEX && from_end <= MAX_WALK_TO_INDEX {
            // Two walks over disjoint ends, each of at most MAX_WALK_TO_INDEX
            // steps -- one iterator driven from both sides would have them meet
            // on a short string.
            let start = self
                .data
                .code_point_indices()
                .nth(range.start)
                .map_or(self.data.len(), |(byte, _)| byte);
            let end = match from_end {
                0 => self.data.len(),
                n => self
                    .data
                    .code_point_indices()
                    .nth_back(n - 1)
                    .map_or(self.data.len(), |(byte, _)| byte),
            };
            return start..end;
        }
        self.char_index_to_byte(range.start)..self.char_index_to_byte(range.end)
    }

    /// The character index of the character starting at byte offset `bytepos`,
    /// the inverse of [`Self::char_index_to_byte`].
    ///
    /// `bytepos` must be a character boundary at or before the end.
    ///
    /// Logarithmic rather than constant, because the index is keyed the other
    /// way -- but a search whose bounds came from `char_index_to_byte` has the
    /// table already, and this is what turns a byte offset back into the answer
    /// a caller asked for in characters.
    pub fn byte_to_char_index(&self, bytepos: usize) -> usize {
        if self.kind().is_ascii() {
            return bytepos;
        }
        let char_len = self.char_len();
        self.index
            .get_or_build(&self.data, char_len)
            .char_index_at_byte(&self.data, bytepos, char_len)
    }

    pub fn nth_char(&self, index: usize) -> CodePoint {
        match self.as_str_kind() {
            PyKindStr::Ascii(s) => s[index].into(),
            _ => self.data[self.char_index_to_byte_once(index)..]
                .code_points()
                .next()
                .unwrap(),
        }
    }
}

impl core::fmt::Display for StrData {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.data.fmt(f)
    }
}

impl CharLen for StrData {
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
#[must_use]
pub fn char_range_end(s: &str, n_chars: usize) -> Option<usize> {
    let i = match n_chars.checked_sub(1) {
        Some(last_char_index) => {
            let (index, c) = s.char_indices().nth(last_char_index)?;
            index + c.len_utf8()
        }
        None => 0,
    };
    Some(i)
}

pub fn try_get_codepoints(w: &Wtf8, range: impl RangeBounds<usize>) -> Option<&Wtf8> {
    let mut chars = w.code_points();
    let start = match range.start_bound() {
        Bound::Included(&i) => i,
        Bound::Excluded(&i) => i + 1,
        Bound::Unbounded => 0,
    };
    for _ in 0..start {
        chars.next()?;
    }
    let s = chars.as_wtf8();
    let range_len = match range.end_bound() {
        Bound::Included(&i) => i + 1 - start,
        Bound::Excluded(&i) => i - start,
        Bound::Unbounded => return Some(s),
    };
    codepoint_range_end(s, range_len).map(|end| &s[..end])
}

pub fn get_codepoints(w: &Wtf8, range: impl RangeBounds<usize>) -> &Wtf8 {
    try_get_codepoints(w, range).unwrap()
}

#[inline]
#[must_use]
pub fn codepoint_range_end(s: &Wtf8, n_chars: usize) -> Option<usize> {
    let i = match n_chars.checked_sub(1) {
        Some(last_char_index) => {
            let (index, c) = s.code_point_indices().nth(last_char_index)?;
            index + c.len_wtf8()
        }
        None => 0,
    };
    Some(i)
}

#[must_use]
/// Returns `None` for a width whose result cannot be allocated.
pub fn zfill(bytes: &[u8], width: usize) -> Option<Vec<u8>> {
    if width <= bytes.len() {
        return Some(bytes.to_vec());
    }
    let (sign, s) = match bytes.first() {
        Some(_sign @ (b'+' | b'-')) => (unsafe { bytes.get_unchecked(..1) }, &bytes[1..]),
        _ => (&b""[..], bytes),
    };
    let mut filled = Vec::new();
    filled.try_reserve_exact(width).ok()?;
    filled.extend_from_slice(sign);
    filled.extend(core::iter::repeat_n(b'0', width - bytes.len()));
    filled.extend_from_slice(s);
    Some(filled)
}

/// Convert a string to ascii compatible, escaping unicode-s into escape
/// sequences.
#[must_use]
pub fn to_ascii(value: &Wtf8) -> AsciiString {
    let mut ascii = Vec::new();
    for cp in value.code_points() {
        if cp.is_ascii() {
            ascii.push(cp.to_u32() as u8);
        } else {
            let c = cp.to_u32();
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

#[derive(Clone, Copy)]
pub struct UnicodeEscapeCodepoint(pub CodePoint);

impl fmt::Display for UnicodeEscapeCodepoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let c = self.0.to_u32();
        if c >= 0x10000 {
            write!(f, "\\U{c:08x}")
        } else if c >= 0x100 {
            write!(f, "\\u{c:04x}")
        } else {
            write!(f, "\\x{c:02x}")
        }
    }
}

pub mod levenshtein {
    pub const MOVE_COST: usize = 2;
    const CASE_COST: usize = 1;
    const MAX_STRING_SIZE: usize = 40;

    const fn substitution_cost(mut a: u8, mut b: u8) -> usize {
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
        if a == b { CASE_COST } else { MOVE_COST }
    }

    #[must_use]
    pub fn levenshtein_distance(a: &[u8], b: &[u8], max_cost: usize) -> usize {
        if a == b {
            return 0;
        }

        let (mut a_bytes, mut b_bytes) = (a, b);
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
            core::mem::swap(&mut a_bytes, &mut b_bytes);
            core::mem::swap(&mut a_begin, &mut b_begin);
            core::mem::swap(&mut a_end, &mut b_end);
        }

        if (b_end - a_end) * MOVE_COST > max_cost {
            return max_cost + 1;
        }

        let mut buffer = [0usize; MAX_STRING_SIZE];

        for (i, x) in buffer.iter_mut().take(a_end).enumerate() {
            *x = (i + 1) * MOVE_COST;
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
    }
}

/// Replace all tabs in a string with spaces, using the given tab size.
#[must_use]
pub fn expandtabs(input: &Wtf8, tab_size: usize) -> Wtf8Buf {
    // A tab size of zero, which is also where a negative one lands, leaves no
    // column for a tab to advance to: the tabs come out and nothing else moves.
    // Going through the arithmetic anyway subtracts the current column from a
    // tab stop of zero and underflows on the first tab, so the width asked for
    // next is `usize::MAX`. The bytes version of this already returns here.
    if tab_size == 0 {
        return input.code_points().filter(|ch| *ch != '\t').collect();
    }

    let tab_stop = tab_size;
    let mut expanded_str = Wtf8Buf::with_capacity(input.len());
    let mut tab_size = tab_stop;
    let mut col_count = 0usize;
    for ch in input.code_points() {
        if ch == '\t' {
            let num_spaces = tab_size - col_count;
            col_count += num_spaces;
            expanded_str.push_str(&" ".repeat(num_spaces));
        } else {
            expanded_str.push(ch);
            if ch == '\r' || ch == '\n' {
                col_count = 0;
                tab_size = 0;
            } else {
                col_count += 1;
            }
        }
        if col_count >= tab_size {
            tab_size += tab_stop;
        }
    }
    expanded_str
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

// TODO: this should probably live in a crate like unic or unicode-properties
const UNICODE_DECIMAL_VALUES: &[char] = &[
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', '٠', '١', '٢', '٣', '٤', '٥', '٦', '٧', '٨',
    '٩', '۰', '۱', '۲', '۳', '۴', '۵', '۶', '۷', '۸', '۹', '߀', '߁', '߂', '߃', '߄', '߅', '߆', '߇',
    '߈', '߉', '०', '१', '२', '३', '४', '५', '६', '७', '८', '९', '০', '১', '২', '৩', '৪', '৫', '৬',
    '৭', '৮', '৯', '੦', '੧', '੨', '੩', '੪', '੫', '੬', '੭', '੮', '੯', '૦', '૧', '૨', '૩', '૪', '૫',
    '૬', '૭', '૮', '૯', '୦', '୧', '୨', '୩', '୪', '୫', '୬', '୭', '୮', '୯', '௦', '௧', '௨', '௩', '௪',
    '௫', '௬', '௭', '௮', '௯', '౦', '౧', '౨', '౩', '౪', '౫', '౬', '౭', '౮', '౯', '೦', '೧', '೨', '೩',
    '೪', '೫', '೬', '೭', '೮', '೯', '൦', '൧', '൨', '൩', '൪', '൫', '൬', '൭', '൮', '൯', '෦', '෧', '෨',
    '෩', '෪', '෫', '෬', '෭', '෮', '෯', '๐', '๑', '๒', '๓', '๔', '๕', '๖', '๗', '๘', '๙', '໐', '໑',
    '໒', '໓', '໔', '໕', '໖', '໗', '໘', '໙', '༠', '༡', '༢', '༣', '༤', '༥', '༦', '༧', '༨', '༩', '၀',
    '၁', '၂', '၃', '၄', '၅', '၆', '၇', '၈', '၉', '႐', '႑', '႒', '႓', '႔', '႕', '႖', '႗', '႘', '႙',
    '០', '១', '២', '៣', '៤', '៥', '៦', '៧', '៨', '៩', '᠐', '᠑', '᠒', '᠓', '᠔', '᠕', '᠖', '᠗', '᠘',
    '᠙', '᥆', '᥇', '᥈', '᥉', '᥊', '᥋', '᥌', '᥍', '᥎', '᥏', '᧐', '᧑', '᧒', '᧓', '᧔', '᧕', '᧖', '᧗',
    '᧘', '᧙', '᪀', '᪁', '᪂', '᪃', '᪄', '᪅', '᪆', '᪇', '᪈', '᪉', '᪐', '᪑', '᪒', '᪓', '᪔', '᪕', '᪖',
    '᪗', '᪘', '᪙', '᭐', '᭑', '᭒', '᭓', '᭔', '᭕', '᭖', '᭗', '᭘', '᭙', '᮰', '᮱', '᮲', '᮳', '᮴', '᮵',
    '᮶', '᮷', '᮸', '᮹', '᱀', '᱁', '᱂', '᱃', '᱄', '᱅', '᱆', '᱇', '᱈', '᱉', '᱐', '᱑', '᱒', '᱓', '᱔',
    '᱕', '᱖', '᱗', '᱘', '᱙', '꘠', '꘡', '꘢', '꘣', '꘤', '꘥', '꘦', '꘧', '꘨', '꘩', '꣐', '꣑', '꣒', '꣓',
    '꣔', '꣕', '꣖', '꣗', '꣘', '꣙', '꤀', '꤁', '꤂', '꤃', '꤄', '꤅', '꤆', '꤇', '꤈', '꤉', '꧐', '꧑', '꧒',
    '꧓', '꧔', '꧕', '꧖', '꧗', '꧘', '꧙', '꧰', '꧱', '꧲', '꧳', '꧴', '꧵', '꧶', '꧷', '꧸', '꧹', '꩐', '꩑',
    '꩒', '꩓', '꩔', '꩕', '꩖', '꩗', '꩘', '꩙', '꯰', '꯱', '꯲', '꯳', '꯴', '꯵', '꯶', '꯷', '꯸', '꯹', '０',
    '１', '２', '３', '４', '５', '６', '７', '８', '９', '𐒠', '𐒡', '𐒢', '𐒣', '𐒤', '𐒥', '𐒦', '𐒧',
    '𐒨', '𐒩', '𑁦', '𑁧', '𑁨', '𑁩', '𑁪', '𑁫', '𑁬', '𑁭', '𑁮', '𑁯', '𑃰', '𑃱', '𑃲', '𑃳', '𑃴', '𑃵', '𑃶',
    '𑃷', '𑃸', '𑃹', '𑄶', '𑄷', '𑄸', '𑄹', '𑄺', '𑄻', '𑄼', '𑄽', '𑄾', '𑄿', '𑇐', '𑇑', '𑇒', '𑇓', '𑇔', '𑇕',
    '𑇖', '𑇗', '𑇘', '𑇙', '𑋰', '𑋱', '𑋲', '𑋳', '𑋴', '𑋵', '𑋶', '𑋷', '𑋸', '𑋹', '𑑐', '𑑑', '𑑒', '𑑓', '𑑔',
    '𑑕', '𑑖', '𑑗', '𑑘', '𑑙', '𑓐', '𑓑', '𑓒', '𑓓', '𑓔', '𑓕', '𑓖', '𑓗', '𑓘', '𑓙', '𑙐', '𑙑', '𑙒', '𑙓',
    '𑙔', '𑙕', '𑙖', '𑙗', '𑙘', '𑙙', '𑛀', '𑛁', '𑛂', '𑛃', '𑛄', '𑛅', '𑛆', '𑛇', '𑛈', '𑛉', '𑜰', '𑜱', '𑜲',
    '𑜳', '𑜴', '𑜵', '𑜶', '𑜷', '𑜸', '𑜹', '𑣠', '𑣡', '𑣢', '𑣣', '𑣤', '𑣥', '𑣦', '𑣧', '𑣨', '𑣩', '𑱐', '𑱑',
    '𑱒', '𑱓', '𑱔', '𑱕', '𑱖', '𑱗', '𑱘', '𑱙', '𑵐', '𑵑', '𑵒', '𑵓', '𑵔', '𑵕', '𑵖', '𑵗', '𑵘', '𑵙', '𖩠',
    '𖩡', '𖩢', '𖩣', '𖩤', '𖩥', '𖩦', '𖩧', '𖩨', '𖩩', '𖭐', '𖭑', '𖭒', '𖭓', '𖭔', '𖭕', '𖭖', '𖭗', '𖭘', '𖭙',
    '𝟎', '𝟏', '𝟐', '𝟑', '𝟒', '𝟓', '𝟔', '𝟕', '𝟖', '𝟗', '𝟘', '𝟙', '𝟚', '𝟛', '𝟜', '𝟝', '𝟞', '𝟟', '𝟠',
    '𝟡', '𝟢', '𝟣', '𝟤', '𝟥', '𝟦', '𝟧', '𝟨', '𝟩', '𝟪', '𝟫', '𝟬', '𝟭', '𝟮', '𝟯', '𝟰', '𝟱', '𝟲', '𝟳',
    '𝟴', '𝟵', '𝟶', '𝟷', '𝟸', '𝟹', '𝟺', '𝟻', '𝟼', '𝟽', '𝟾', '𝟿', '𞥐', '𞥑', '𞥒', '𞥓', '𞥔', '𞥕', '𞥖',
    '𞥗', '𞥘', '𞥙',
];

#[must_use]
pub fn char_to_decimal(ch: char) -> Option<u8> {
    UNICODE_DECIMAL_VALUES
        .binary_search(&ch)
        .ok()
        .map(|i| (i % 10) as u8)
}

/// Replace Unicode decimal digits with their ASCII equivalents and any Unicode
/// whitespace with a plain space, so the byte-oriented numeric parsers can read
/// them. Mirrors CPython's `_PyUnicode_TransformDecimalAndSpaceToASCII`.
///
/// The result is always ASCII. Any other non-ASCII character cannot appear in a
/// numeric literal, so it becomes a `?` and the rest of the string is dropped:
/// `?` is rejected by every parser at every base, which leaves the caller — the
/// one that knows the base and owns the original string — to raise the error.
#[must_use]
pub fn transform_decimal_and_space_to_ascii(s: &str) -> Cow<'_, str> {
    if s.is_ascii() {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if (c as u32) < 127 {
            out.push(c);
        } else if c.is_whitespace() {
            out.push(' ');
        } else if let Some(n) = char_to_decimal(c) {
            out.push(char::from_digit(n.into(), 10).unwrap());
        } else {
            out.push('?');
            break;
        }
    }
    debug_assert!(out.is_ascii());
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const _: () = assert!(core::mem::size_of::<StrData>() == 5 * core::mem::size_of::<usize>());

    #[test]
    fn str_data_append_updates_value_kind_and_length() {
        let mut data = StrData::from(Box::<str>::from("ascii"));
        data.append(Wtf8::new("é"));
        assert_eq!(data.as_wtf8(), Wtf8::new("ascii\u{e9}"));
        assert_eq!(data.kind(), StrKind::Utf8);
        assert_eq!(data.char_len(), 6);

        let surrogate = Wtf8Buf::from(CodePoint::from_u32(0xd800).unwrap());
        data.append(&surrogate);
        assert_eq!(data.kind(), StrKind::Wtf8);
        assert_eq!(data.char_len(), 7);
    }

    #[test]
    fn str_data_append_reuses_reserved_capacity() {
        let mut data = StrData::from(Box::<str>::from("a"));
        data.append(Wtf8::new("b"));
        let capacity = data.data.capacity();

        while data.len() < capacity {
            data.append(Wtf8::new("c"));
            assert_eq!(data.data.capacity(), capacity);
        }
    }

    #[test]
    fn transform_decimal_and_space() {
        // ASCII input is passed through untouched, without allocating.
        assert!(matches!(
            transform_decimal_and_space_to_ascii("123"),
            Cow::Borrowed("123")
        ));
        // Decimal digits from any script fold to ASCII.
        assert_eq!(transform_decimal_and_space_to_ascii("١٢٣"), "123");
        assert_eq!(transform_decimal_and_space_to_ascii("１２३"), "123");
        assert_eq!(transform_decimal_and_space_to_ascii("1٢3"), "123");
        // Unicode whitespace folds to a plain space.
        assert_eq!(transform_decimal_and_space_to_ascii("\u{3000}٣"), " 3");
        // ASCII characters ride through untouched, whatever they are.
        assert_eq!(transform_decimal_and_space_to_ascii("0x١f"), "0x1f");
        assert_eq!(transform_decimal_and_space_to_ascii("-١_٢"), "-1_2");
        // Anything else poisons the literal and truncates it, so the result stays
        // ASCII and the caller's parser is guaranteed to reject it.
        assert_eq!(transform_decimal_and_space_to_ascii("½가"), "?");
        assert_eq!(transform_decimal_and_space_to_ascii("١٢가٣"), "12?");
        assert_eq!(transform_decimal_and_space_to_ascii("١\u{7f}"), "1?");
    }

    #[test]
    fn get_chars_basic() {
        let s = "0123456789";
        assert_eq!(get_chars(s, 3..7), "3456");
        assert_eq!(get_chars(s, 3..7), &s[3..7]);

        let s = "0유니코드 문자열9";
        assert_eq!(get_chars(s, 3..7), "코드 문");

        let s = "0😀😃😄😁😆😅😂🤣9";
        assert_eq!(get_chars(s, 3..7), "😄😁😆😅");
    }

    fn expandtabs(input: &str, tab_size: usize) -> Wtf8Buf {
        super::expandtabs(Wtf8::new(input), tab_size)
    }

    #[test]
    fn expandtabs_with_zero_tab_size_drops_tabs() {
        // A tab that follows a character used to subtract that column from a
        // tab stop of zero, so the width of the run of spaces came out as
        // `usize::MAX` and the allocation aborted the process.
        assert_eq!(expandtabs("a\tb", 0), Wtf8Buf::from("ab"));
        assert_eq!(expandtabs("ab\tcd\tef", 0), Wtf8Buf::from("abcdef"));
        assert_eq!(expandtabs("a\nb\tc", 0), Wtf8Buf::from("a\nbc"));
        assert_eq!(expandtabs("á\tb", 0), Wtf8Buf::from("áb"));
        assert_eq!(expandtabs("\ta", 0), Wtf8Buf::from("a"));
        assert_eq!(expandtabs("\t", 0), Wtf8Buf::from(""));
        assert_eq!(expandtabs("", 0), Wtf8Buf::from(""));
        assert_eq!(expandtabs("no tabs", 0), Wtf8Buf::from("no tabs"));
    }

    #[test]
    fn expandtabs_with_a_real_tab_size_is_unchanged() {
        assert_eq!(expandtabs("a\tb", 8), Wtf8Buf::from("a       b"));
        assert_eq!(expandtabs("a\tb", 1), Wtf8Buf::from("a b"));
        assert_eq!(expandtabs("abcd\te", 4), Wtf8Buf::from("abcd    e"));
        assert_eq!(expandtabs("a\nb\tc", 4), Wtf8Buf::from("a\nb   c"));
        assert_eq!(expandtabs("\ta", 4), Wtf8Buf::from("    a"));
    }
}
