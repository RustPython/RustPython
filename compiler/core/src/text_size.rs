#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use std::cmp;
use std::cmp::Ordering;
use std::convert::TryFrom;
use std::num::TryFromIntError;
use std::ops::{Add, AddAssign, Bound, Index, IndexMut, Range, RangeBounds, Sub, SubAssign};

pub type Location = TextSize;

/// Source code location.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TextSize(u32);

impl TextSize {
    pub fn fmt_with(
        &self,
        f: &mut std::fmt::Formatter,
        e: &impl std::fmt::Display,
    ) -> std::fmt::Result {
        write!(f, "{} at offset {}", e, self.0)
    }

    #[inline]
    pub const fn new(offset: u32) -> Self {
        Self(offset)
    }

    #[inline]
    pub const fn zero() -> Self {
        Self(0)
    }

    #[inline]
    pub fn newline(&mut self) {
        self.0 += 1;
    }

    #[inline]
    pub fn of<T>(text_len: T) -> Self
    where
        T: TextLen,
    {
        text_len.text_len()
    }

    #[inline]
    pub const fn saturating_sub(self, rhs: TextSize) -> TextSize {
        TextSize(self.0.saturating_sub(rhs.0))
    }

    #[inline]
    pub const fn saturating_add(self, rhs: TextSize) -> TextSize {
        TextSize(self.0.saturating_add(rhs.0))
    }

    #[inline]
    pub fn checked_add(self, rhs: TextSize) -> Option<TextSize> {
        self.0.checked_add(rhs.0).map(TextSize)
    }

    /// Checked subtraction. Returns `None` if overflow occurred.
    #[inline]
    pub fn checked_sub(self, rhs: TextSize) -> Option<TextSize> {
        self.0.checked_sub(rhs.0).map(TextSize)
    }
}

impl TryFrom<usize> for TextSize {
    type Error = TryFromIntError;

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        Ok(Location::new(u32::try_from(value)?))
    }
}

impl From<u32> for Location {
    fn from(value: u32) -> Self {
        TextSize(value)
    }
}

impl From<Location> for u32 {
    fn from(value: TextSize) -> Self {
        value.0
    }
}

impl From<TextSize> for usize {
    fn from(value: TextSize) -> Self {
        value.0 as usize
    }
}

impl Add<TextSize> for TextSize {
    type Output = TextSize;

    #[inline]
    fn add(self, rhs: TextSize) -> Self::Output {
        TextSize(self.0 + rhs.0)
    }
}

impl Add<TextSize> for &TextSize {
    type Output = TextSize;

    fn add(self, rhs: TextSize) -> Self::Output {
        TextSize(self.0 + rhs.0)
    }
}

impl Add<&TextSize> for TextSize {
    type Output = TextSize;

    fn add(self, rhs: &TextSize) -> Self::Output {
        TextSize(self.0 + rhs.0)
    }
}

impl Sub<TextSize> for TextSize {
    type Output = TextSize;

    #[inline]
    fn sub(self, rhs: TextSize) -> Self::Output {
        TextSize(self.0 - rhs.0)
    }
}

impl Sub<TextSize> for &TextSize {
    type Output = TextSize;

    fn sub(self, rhs: TextSize) -> Self::Output {
        TextSize(self.0 - rhs.0)
    }
}

impl Sub<&TextSize> for TextSize {
    type Output = TextSize;

    fn sub(self, rhs: &TextSize) -> Self::Output {
        TextSize(self.0 - rhs.0)
    }
}

impl<A> std::ops::AddAssign<A> for TextSize
where
    TextSize: Add<A, Output = TextSize>,
{
    #[inline]
    fn add_assign(&mut self, rhs: A) {
        *self = *self + rhs;
    }
}

impl<S> std::ops::SubAssign<S> for TextSize
where
    TextSize: Sub<S, Output = TextSize>,
{
    #[inline]
    fn sub_assign(&mut self, rhs: S) {
        *self = *self - rhs;
    }
}

pub trait TextLen {
    fn text_len(&self) -> TextSize;
}

impl TextLen for &str {
    fn text_len(&self) -> TextSize {
        TextSize::try_from(self.len()).unwrap()
    }
}

impl TextLen for String {
    fn text_len(&self) -> TextSize {
        self.as_str().text_len()
    }
}

impl TextLen for char {
    fn text_len(&self) -> TextSize {
        TextSize::new(self.len_utf8() as u32)
    }
}

#[derive(Copy, Clone, Eq, PartialEq, Hash, Default)]
pub struct TextRange {
    start: TextSize,
    end: TextSize,
}

impl TextRange {
    #[inline]
    pub fn new(start: TextSize, end: TextSize) -> Self {
        assert!(start <= end);

        Self { start, end }
    }

    #[inline]
    pub fn at(offset: TextSize, length: TextSize) -> Self {
        Self {
            start: offset,
            end: offset + length,
        }
    }

    #[inline]
    pub fn empty(offset: TextSize) -> Self {
        Self {
            start: offset,
            end: offset,
        }
    }

    #[inline]
    pub fn up_to(end: TextSize) -> Self {
        Self {
            start: TextSize::zero(),
            end,
        }
    }

    #[inline]
    pub const fn start(self) -> TextSize {
        self.start
    }

    #[inline]
    pub const fn end(self) -> TextSize {
        self.end
    }

    #[inline]
    pub const fn len(self) -> TextSize {
        TextSize::new(self.end.0 - self.start.0)
    }

    #[inline]
    pub const fn is_empty(self) -> bool {
        self.len().0 == 0
    }

    #[inline]
    pub const fn contains(self, offset: TextSize) -> bool {
        self.start.0 <= offset.0 && offset.0 < self.end.0
    }

    #[inline]
    pub const fn contains_inclusive(self, offset: TextSize) -> bool {
        self.start.0 <= offset.0 && offset.0 <= self.end.0
    }

    #[inline]
    pub const fn contains_range(self, other: TextRange) -> bool {
        self.start.0 <= other.start.0 && other.end.0 <= self.end.0
    }

    /// The range covered by both ranges, if it exists.
    /// If the ranges touch but do not overlap, the output range is empty.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use text_size::*;
    /// assert_eq!(
    ///     TextRange::intersect(
    ///         TextRange::new(0.into(), 10.into()),
    ///         TextRange::new(5.into(), 15.into()),
    ///     ),
    ///     Some(TextRange::new(5.into(), 10.into())),
    /// );
    /// ```
    #[inline]
    pub fn intersect(self, other: TextRange) -> Option<TextRange> {
        let start = cmp::max(self.start(), other.start());
        let end = cmp::min(self.end(), other.end());
        if end < start {
            return None;
        }
        Some(TextRange::new(start, end))
    }

    /// Extends the range to cover `other` as well.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use text_size::*;
    /// assert_eq!(
    ///     TextRange::cover(
    ///         TextRange::new(0.into(), 5.into()),
    ///         TextRange::new(15.into(), 20.into()),
    ///     ),
    ///     TextRange::new(0.into(), 20.into()),
    /// );
    /// ```
    #[inline]
    pub fn cover(self, other: TextRange) -> TextRange {
        let start = cmp::min(self.start(), other.start());
        let end = cmp::max(self.end(), other.end());
        TextRange::new(start, end)
    }

    /// Extends the range to cover `other` offsets as well.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use text_size::*;
    /// assert_eq!(
    ///     TextRange::empty(0.into()).cover_offset(20.into()),
    ///     TextRange::new(0.into(), 20.into()),
    /// )
    /// ```
    #[inline]
    pub fn cover_offset(self, offset: TextSize) -> TextRange {
        self.cover(TextRange::empty(offset))
    }

    /// Add an offset to this range.
    ///
    /// Note that this is not appropriate for changing where a `TextRange` is
    /// within some string; rather, it is for changing the reference anchor
    /// that the `TextRange` is measured against.
    ///
    /// The unchecked version (`Add::add`) will _always_ panic on overflow,
    /// in contrast to primitive integers, which check in debug mode only.
    #[inline]
    pub fn checked_add(self, offset: TextSize) -> Option<TextRange> {
        Some(TextRange {
            start: self.start.checked_add(offset)?,
            end: self.end.checked_add(offset)?,
        })
    }

    /// Subtract an offset from this range.
    ///
    /// Note that this is not appropriate for changing where a `TextRange` is
    /// within some string; rather, it is for changing the reference anchor
    /// that the `TextRange` is measured against.
    ///
    /// The unchecked version (`Sub::sub`) will _always_ panic on overflow,
    /// in contrast to primitive integers, which check in debug mode only.
    #[inline]
    pub fn checked_sub(self, offset: TextSize) -> Option<TextRange> {
        Some(TextRange {
            start: self.start.checked_sub(offset)?,
            end: self.end.checked_sub(offset)?,
        })
    }

    /// Relative order of the two ranges (overlapping ranges are considered
    /// equal).
    ///
    ///
    /// This is useful when, for example, binary searching an array of disjoint
    /// ranges.
    ///
    /// # Examples
    ///
    /// ```
    /// # use text_size::*;
    /// # use std::cmp::Ordering;
    ///
    /// let a = TextRange::new(0.into(), 3.into());
    /// let b = TextRange::new(4.into(), 5.into());
    /// assert_eq!(a.ordering(b), Ordering::Less);
    ///
    /// let a = TextRange::new(0.into(), 3.into());
    /// let b = TextRange::new(3.into(), 5.into());
    /// assert_eq!(a.ordering(b), Ordering::Less);
    ///
    /// let a = TextRange::new(0.into(), 3.into());
    /// let b = TextRange::new(2.into(), 5.into());
    /// assert_eq!(a.ordering(b), Ordering::Equal);
    ///
    /// let a = TextRange::new(0.into(), 3.into());
    /// let b = TextRange::new(2.into(), 2.into());
    /// assert_eq!(a.ordering(b), Ordering::Equal);
    ///
    /// let a = TextRange::new(2.into(), 3.into());
    /// let b = TextRange::new(2.into(), 2.into());
    /// assert_eq!(a.ordering(b), Ordering::Greater);
    /// ```
    #[inline]
    pub fn ordering(self, other: TextRange) -> Ordering {
        if self.end() <= other.start() {
            Ordering::Less
        } else if other.end() <= self.start() {
            Ordering::Greater
        } else {
            Ordering::Equal
        }
    }
}

impl Index<TextRange> for str {
    type Output = str;

    fn index(&self, index: TextRange) -> &Self::Output {
        &self[usize::from(index.start)..usize::from(index.end)]
    }
}

impl IndexMut<TextRange> for str {
    fn index_mut(&mut self, index: TextRange) -> &mut Self::Output {
        &mut self[usize::from(index.start)..usize::from(index.end)]
    }
}

impl Index<TextRange> for String {
    type Output = str;

    fn index(&self, index: TextRange) -> &Self::Output {
        &self[usize::from(index.start)..usize::from(index.end)]
    }
}

impl IndexMut<TextRange> for String {
    fn index_mut(&mut self, index: TextRange) -> &mut Self::Output {
        &mut self[usize::from(index.start)..usize::from(index.end)]
    }
}

impl RangeBounds<TextSize> for TextRange {
    fn start_bound(&self) -> Bound<&TextSize> {
        Bound::Included(&self.start)
    }

    fn end_bound(&self) -> Bound<&TextSize> {
        Bound::Excluded(&self.end)
    }
}

impl<T> From<TextRange> for Range<T>
where
    T: From<TextSize>,
{
    #[inline]
    fn from(r: TextRange) -> Self {
        r.start().into()..r.end().into()
    }
}

impl Add<TextSize> for TextRange {
    type Output = TextRange;
    #[inline]
    fn add(self, offset: TextSize) -> TextRange {
        self.checked_add(offset)
            .expect("TextRange +offset overflowed")
    }
}

impl Add<TextSize> for &TextRange {
    type Output = TextRange;
    #[inline]
    fn add(self, offset: TextSize) -> TextRange {
        *self + offset
    }
}

impl Sub<TextSize> for TextRange {
    type Output = TextRange;
    #[inline]
    fn sub(self, offset: TextSize) -> TextRange {
        self.checked_sub(offset)
            .expect("TextRange +offset overflowed")
    }
}

impl Sub<TextSize> for &TextRange {
    type Output = TextRange;
    #[inline]
    fn sub(self, offset: TextSize) -> TextRange {
        *self - offset
    }
}

impl<A> AddAssign<A> for TextRange
where
    TextRange: Add<A, Output = TextRange>,
{
    #[inline]
    fn add_assign(&mut self, rhs: A) {
        *self = *self + rhs
    }
}

impl<S> SubAssign<S> for TextRange
where
    TextRange: Sub<S, Output = TextRange>,
{
    #[inline]
    fn sub_assign(&mut self, rhs: S) {
        *self = *self - rhs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
}
