// spell-checker:disable
//! Modified from core::str::count

use super::Wtf8;

const USIZE_SIZE: usize = core::mem::size_of::<usize>();
const UNROLL_INNER: usize = 4;

#[inline]
pub(super) fn count_chars(s: &Wtf8) -> usize {
    if s.len() < USIZE_SIZE * UNROLL_INNER {
        // Avoid entering the optimized implementation for strings where the
        // difference is not likely to matter, or where it might even be slower.
        // That said, a ton of thought was not spent on the particular threshold
        // here, beyond "this value seems to make sense".
        char_count_general_case(s.as_bytes())
    } else {
        do_count_chars(s)
    }
}

fn do_count_chars(s: &Wtf8) -> usize {
    // For correctness, `CHUNK_SIZE` must be:
    //
    // - Less than or equal to 255, otherwise we'll overflow bytes in `counts`.
    // - A multiple of `UNROLL_INNER`, otherwise our `break` inside the
    //   `body.chunks(CHUNK_SIZE)` loop is incorrect.
    //
    // For performance, `CHUNK_SIZE` should be:
    // - Relatively cheap to `/` against (so some simple sum of powers of two).
    // - Large enough to avoid paying for the cost of the `sum_bytes_in_usize`
    //   too often.
    const CHUNK_SIZE: usize = 192;

    // Check the properties of `CHUNK_SIZE` and `UNROLL_INNER` that are required
    // for correctness.
    const _: () = assert!(CHUNK_SIZE < 256);
    const _: () = assert!(CHUNK_SIZE.is_multiple_of(UNROLL_INNER));

    // SAFETY: transmuting `[u8]` to `[usize]` is safe except for size
    // differences which are handled by `align_to`.
    let (head, body, tail) = unsafe { s.as_bytes().align_to::<usize>() };

    // This should be quite rare, and basically exists to handle the degenerate
    // cases where align_to fails (as well as miri under symbolic alignment
    // mode).
    //
    // The `unlikely` helps discourage LLVM from inlining the body, which is
    // nice, as we would rather not mark the `char_count_general_case` function
    // as cold.
    if unlikely(body.is_empty() || head.len() > USIZE_SIZE || tail.len() > USIZE_SIZE) {
        return char_count_general_case(s.as_bytes());
    }

    let mut total = char_count_general_case(head) + char_count_general_case(tail);
    // Split `body` into `CHUNK_SIZE` chunks to reduce the frequency with which
    // we call `sum_bytes_in_usize`.
    for chunk in body.chunks(CHUNK_SIZE) {
        // We accumulate intermediate sums in `counts`, where each byte contains
        // a subset of the sum of this chunk, like a `[u8; size_of::<usize>()]`.
        let mut counts = 0;

        let (unrolled_chunks, remainder) = slice_as_chunks::<_, UNROLL_INNER>(chunk);
        for unrolled in unrolled_chunks {
            for &word in unrolled {
                // Because `CHUNK_SIZE` is < 256, this addition can't cause the
                // count in any of the bytes to overflow into a subsequent byte.
                counts += contains_non_continuation_byte(word);
            }
        }

        // Sum the values in `counts` (which, again, is conceptually a `[u8;
        // size_of::<usize>()]`), and accumulate the result into `total`.
        total += sum_bytes_in_usize(counts);

        // If there's any data in `remainder`, then handle it. This will only
        // happen for the last `chunk` in `body.chunks()` (because `CHUNK_SIZE`
        // is divisible by `UNROLL_INNER`), so we explicitly break at the end
        // (which seems to help LLVM out).
        if !remainder.is_empty() {
            // Accumulate all the data in the remainder.
            let mut counts = 0;
            for &word in remainder {
                counts += contains_non_continuation_byte(word);
            }
            total += sum_bytes_in_usize(counts);
            break;
        }
    }
    total
}

// Checks each byte of `w` to see if it contains the first byte in a UTF-8
// sequence. Bytes in `w` which are continuation bytes are left as `0x00` (e.g.
// false), and bytes which are non-continuation bytes are left as `0x01` (e.g.
// true)
#[inline]
fn contains_non_continuation_byte(w: usize) -> usize {
    const LSB: usize = usize_repeat_u8(0x01);
    ((!w >> 7) | (w >> 6)) & LSB
}

// Morally equivalent to `values.to_ne_bytes().into_iter().sum::<usize>()`, but
// more efficient.
#[inline]
fn sum_bytes_in_usize(values: usize) -> usize {
    const LSB_SHORTS: usize = usize_repeat_u16(0x0001);
    const SKIP_BYTES: usize = usize_repeat_u16(0x00ff);

    let pair_sum: usize = (values & SKIP_BYTES) + ((values >> 8) & SKIP_BYTES);
    pair_sum.wrapping_mul(LSB_SHORTS) >> ((USIZE_SIZE - 2) * 8)
}

// This is the most direct implementation of the concept of "count the number of
// bytes in the string which are not continuation bytes", and is used for the
// head and tail of the input string (the first and last item in the tuple
// returned by `slice::align_to`).
fn char_count_general_case(s: &[u8]) -> usize {
    s.iter()
        .filter(|&&byte| !super::core_str::utf8_is_cont_byte(byte))
        .count()
}

// polyfills of unstable library features

const fn usize_repeat_u8(x: u8) -> usize {
    usize::from_ne_bytes([x; size_of::<usize>()])
}

const fn usize_repeat_u16(x: u16) -> usize {
    let mut r = 0usize;
    let mut i = 0;
    while i < size_of::<usize>() {
        // Use `wrapping_shl` to make it work on targets with 16-bit `usize`
        r = r.wrapping_shl(16) | (x as usize);
        i += 2;
    }
    r
}

fn slice_as_chunks<T, const N: usize>(slice: &[T]) -> (&[[T; N]], &[T]) {
    assert!(N != 0, "chunk size must be non-zero");
    let len_rounded_down = slice.len() / N * N;
    // SAFETY: The rounded-down value is always the same or smaller than the
    // original length, and thus must be in-bounds of the slice.
    let (multiple_of_n, remainder) = unsafe { slice.split_at_unchecked(len_rounded_down) };
    // SAFETY: We already panicked for zero, and ensured by construction
    // that the length of the subslice is a multiple of N.
    let array_slice = unsafe { slice_as_chunks_unchecked(multiple_of_n) };
    (array_slice, remainder)
}

unsafe fn slice_as_chunks_unchecked<T, const N: usize>(slice: &[T]) -> &[[T; N]] {
    let new_len = slice.len() / N;
    // SAFETY: We cast a slice of `new_len * N` elements into
    // a slice of `new_len` many `N` elements chunks.
    unsafe { std::slice::from_raw_parts(slice.as_ptr().cast(), new_len) }
}

const fn unlikely(x: bool) -> bool {
    x
}
