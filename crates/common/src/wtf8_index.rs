// spell-checker:ignore rpython rlib rutf
//! Random access into a WTF-8 buffer.
//!
//! WTF-8 is variable width, so a buffer's n-th code point can only be found by
//! decoding the n-1 before it: [`Wtf8`]'s iterators are sequential, and
//! resolving an index through them is O(n). Code that indexes the same string
//! repeatedly -- a regex scan restarting at successive positions, say -- then
//! walks the whole buffer once per index, which is quadratic in its length.
//!
//! [`Wtf8Index`] is the side table that makes the lookup O(1): one 24-byte
//! group per 64 code points, so 0.375 bytes per code point. It is a cache, and
//! holds no state of its own beyond the buffer's shape -- building it twice for
//! the same buffer yields the same table.
//!
//! The layout is PyPy's `UTF8_INDEX_STORAGE` (`rpython/rlib/rutf8.py`).

use crate::wtf8::Wtf8;

/// One group of 64 code points.
#[derive(Clone, Copy)]
struct Group {
    /// The byte offset the group's first code point starts at.
    base: usize,
    /// `ofs[i]` is the byte offset of the group's `4 * i + 1`-th code point,
    /// relative to `base`. One entry covers four code points, so the widest
    /// offset an entry has to hold is that of the 61st code point of a group,
    /// at most `61 * 4 = 244` bytes in -- inside a `u8`, which is what buys the
    /// table its density.
    ofs: [u8; 16],
}

/// A code-point-index to byte-offset table for one WTF-8 buffer.
pub struct Wtf8Index {
    groups: Box<[Group]>,
}

impl Wtf8Index {
    /// Builds the table for `data`, whose code point count is `char_len`.
    ///
    /// O(`data.len()`), and touches every byte, so it pays for itself only when
    /// the caller goes on to index the buffer more than a couple of times.
    #[must_use]
    pub fn new(data: &Wtf8, char_len: usize) -> Self {
        let mut groups = vec![
            Group {
                base: 0,
                ofs: [0; 16],
            };
            char_len / 64 + 1
        ];
        // Signed: the countdown overshoots the last group -- the loop stops on
        // the first negative value rather than at a group boundary.
        let mut remaining = char_len as isize;
        let mut base = 0;
        let mut current = 0;
        loop {
            groups[current].base = base;
            let mut next = base;
            let mut group_filled = true;
            for i in 0..16 {
                // Past the end, step as if one more single-byte code point
                // followed, so the entry stays in range and is never read.
                next = if remaining == 0 {
                    next + 1
                } else {
                    next_pos(data, next)
                };
                groups[current].ofs[i] = (next - base) as u8;
                remaining -= 4;
                if remaining < 0 {
                    debug_assert_eq!(current + 1, groups.len());
                    group_filled = false;
                    break;
                }
                next = next_pos(data, next_pos(data, next_pos(data, next)));
            }
            if !group_filled {
                break;
            }
            current += 1;
            base = next;
        }
        Self {
            groups: groups.into_boxed_slice(),
        }
    }

    /// The byte offset of `data`'s `index`-th code point.
    ///
    /// `data` must be the buffer the table was built for, and `index` must be
    /// below its code point count.
    #[inline]
    #[must_use]
    pub fn byte_offset(&self, data: &Wtf8, index: usize) -> usize {
        let group = &self.groups[index >> 6];
        // The entry sits on the 4k+1-th code point of the group, so a lookup is
        // one table read plus at most two steps in either direction.
        let pos = group.base + group.ofs[(index >> 2) & 0x0F] as usize;
        match index & 0x3 {
            0 => prev_pos(data, pos),
            1 => pos,
            2 => next_pos(data, pos),
            _ => next_pos(data, next_pos(data, pos)),
        }
    }

    /// The index of the code point starting at byte offset `bytepos`, the
    /// inverse of [`Self::byte_offset`].
    ///
    /// `data` must be the buffer the table was built for, `char_len` its code
    /// point count, and `bytepos` a code point boundary at or before its end.
    ///
    /// Logarithmic rather than constant: the table is keyed by code point
    /// index, so going the other way is a search through it. The bracketing
    /// below is what keeps that search short -- a code point occupies one to
    /// four bytes, which pins the answer to a narrow band around `bytepos`
    /// before the first comparison.
    #[must_use]
    pub fn char_index_at_byte(&self, data: &Wtf8, bytepos: usize, char_len: usize) -> usize {
        let bytes_remaining = data.len() - bytepos;
        // At least one byte per remaining code point, and at most four, so the
        // group holding the answer lies between these.
        let mut group_min =
            usize::max(bytepos / 4, char_len.saturating_sub(bytes_remaining + 1)) >> 6;
        let mut group_max = usize::min(bytepos, char_len.saturating_sub(bytes_remaining / 4)) >> 6;
        while group_min < group_max {
            let middle = group_min.midpoint(group_max) + 1;
            if bytepos < self.groups[middle].base {
                group_max = middle - 1;
            } else {
                group_min = middle;
            }
        }

        let base = self.groups[group_min].base;
        if base == bytepos {
            return group_min << 6;
        }
        // Walk the group's entries to the last one at or before `bytepos`,
        // then step the remaining code points, of which there are at most
        // three -- an entry covers four.
        let entries = if group_min == self.groups.len() - 1 {
            ((char_len - 1) >> 2) & 0x0F
        } else {
            16
        };
        let mut index = group_min << 6;
        let mut pos = base;
        for entry in 0..entries {
            let at = base + self.groups[group_min].ofs[entry] as usize;
            if at >= bytepos {
                break;
            }
            pos = at;
            index = (group_min << 6) + (entry << 2) + 1;
        }
        while pos < bytepos {
            pos = next_pos(data, pos);
            index += 1;
        }
        index
    }

    /// The table's heap footprint, in bytes.
    #[must_use]
    pub fn byte_size(&self) -> usize {
        core::mem::size_of_val(&*self.groups)
    }
}

/// The byte offset of the code point after the one at `pos`.
///
/// `data` must be well-formed WTF-8 and `pos` a code point boundary before its
/// end -- reading only the lead byte is what makes this branch-light.
#[inline]
fn next_pos(data: &Wtf8, pos: usize) -> usize {
    match data.as_bytes()[pos] {
        0x00..=0x7F => pos + 1,
        0x80..=0xDF => pos + 2,
        0xE0..=0xEF => pos + 3,
        _ => pos + 4,
    }
}

/// The byte offset of the code point before the one at `pos`, which must not be
/// zero.
///
/// A `pos` one past the end reads as the extra code point [`Wtf8Index::new`]
/// steps over there.
#[inline]
fn prev_pos(data: &Wtf8, pos: usize) -> usize {
    let data = data.as_bytes();
    let mut pos = pos - 1;
    if pos >= data.len() || data[pos] <= 0x7F {
        return pos;
    }
    pos -= 1;
    if data[pos] >= 0xC0 {
        return pos;
    }
    pos -= 1;
    if data[pos] >= 0xC0 {
        return pos;
    }
    pos - 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wtf8::{CodePoint, Wtf8Buf};

    /// Every index of `s`, both ways, against the offsets its own iterator
    /// reports.
    fn check(s: &Wtf8) {
        let expected: Vec<usize> = s
            .code_point_indices()
            .map(|(byte_offset, _)| byte_offset)
            .collect();
        let char_len = expected.len();
        let index = Wtf8Index::new(s, char_len);
        for (i, &want) in expected.iter().enumerate() {
            assert_eq!(
                index.byte_offset(s, i),
                want,
                "index {i} of {s:?} ({char_len} code points)"
            );
            assert_eq!(
                index.char_index_at_byte(s, want, char_len),
                i,
                "byte {want} of {s:?} ({char_len} code points)"
            );
        }
        // One past the last code point is a boundary too, and the searches that
        // use this ask for it as an end bound.
        assert_eq!(
            index.char_index_at_byte(s, s.len(), char_len),
            char_len,
            "end of {s:?}"
        );
    }

    fn wtf8(s: &str) -> Wtf8Buf {
        Wtf8Buf::from(s)
    }

    #[test]
    fn empty() {
        check(wtf8("").as_ref());
    }

    #[test]
    fn widths() {
        // One case per encoded width, and the boundaries between them.
        check(wtf8("abc").as_ref());
        check(wtf8("\u{80}\u{7ff}").as_ref());
        check(wtf8("\u{800}\u{ffff}").as_ref());
        check(wtf8("\u{10000}\u{10ffff}").as_ref());
        check(wtf8("a\u{80}\u{800}\u{10000}").as_ref());
    }

    #[test]
    fn group_boundaries() {
        // A group covers 64 code points and an entry four, so the interesting
        // lengths are the ones on and around both.
        for len in [1, 3, 4, 5, 63, 64, 65, 127, 128, 129, 255, 256, 257] {
            for unit in ["a", "\u{80}", "\u{800}", "\u{10000}"] {
                check(wtf8(&unit.repeat(len)).as_ref());
            }
            // Mixed widths, so a group's entries do not share a stride.
            check(wtf8(&"a\u{80}\u{800}\u{10000}".repeat(len)).as_ref());
        }
    }

    #[test]
    fn lone_surrogates() {
        let mut s = wtf8("a");
        for cp in [0xD800, 0xDBFF, 0xDC00, 0xDFFF] {
            s.push(CodePoint::from_u32(cp).unwrap());
            s.push_str("b");
        }
        check(s.as_ref());

        // Surrogates only, spanning more than one group.
        let mut s = wtf8("");
        for i in 0..200 {
            s.push(CodePoint::from_u32(0xD800 + (i % 0x400)).unwrap());
        }
        check(s.as_ref());
    }

    #[test]
    fn byte_size_is_one_group_per_64_code_points() {
        let s = wtf8(&"\u{10000}".repeat(200));
        let index = Wtf8Index::new(s.as_ref(), 200);
        assert_eq!(index.byte_size(), (200 / 64 + 1) * size_of::<Group>());
        assert_eq!(size_of::<Group>(), 24);
    }
}
