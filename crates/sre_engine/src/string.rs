use rustpython_wtf8::Wtf8;

#[derive(Debug, Clone, Copy)]
pub struct StringCursor {
    pub(crate) ptr: *const u8,
    pub position: usize,
}

impl Default for StringCursor {
    fn default() -> Self {
        Self {
            ptr: core::ptr::null(),
            position: 0,
        }
    }
}

pub trait StrDrive: Copy {
    fn count(&self) -> usize;
    fn create_cursor(&self, n: usize) -> StringCursor;
    fn adjust_cursor(&self, cursor: &mut StringCursor, n: usize);
    fn advance(cursor: &mut StringCursor) -> u32;
    fn peek(cursor: &StringCursor) -> u32;
    fn skip(cursor: &mut StringCursor, n: usize);
    fn back_advance(cursor: &mut StringCursor) -> u32;
    fn back_peek(cursor: &StringCursor) -> u32;
    fn back_skip(cursor: &mut StringCursor, n: usize);
}

impl StrDrive for &[u8] {
    #[inline]
    fn count(&self) -> usize {
        self.len()
    }

    #[inline]
    fn create_cursor(&self, n: usize) -> StringCursor {
        StringCursor {
            ptr: self[n..].as_ptr(),
            position: n,
        }
    }

    #[inline]
    fn adjust_cursor(&self, cursor: &mut StringCursor, n: usize) {
        cursor.position = n;
        cursor.ptr = self[n..].as_ptr();
    }

    #[inline]
    fn advance(cursor: &mut StringCursor) -> u32 {
        cursor.position += 1;
        unsafe { cursor.ptr = cursor.ptr.add(1) };
        unsafe { *cursor.ptr as u32 }
    }

    #[inline]
    fn peek(cursor: &StringCursor) -> u32 {
        unsafe { *cursor.ptr as u32 }
    }

    #[inline]
    fn skip(cursor: &mut StringCursor, n: usize) {
        cursor.position += n;
        unsafe { cursor.ptr = cursor.ptr.add(n) };
    }

    #[inline]
    fn back_advance(cursor: &mut StringCursor) -> u32 {
        cursor.position -= 1;
        unsafe { cursor.ptr = cursor.ptr.sub(1) };
        unsafe { *cursor.ptr as u32 }
    }

    #[inline]
    fn back_peek(cursor: &StringCursor) -> u32 {
        unsafe { *cursor.ptr.offset(-1) as u32 }
    }

    #[inline]
    fn back_skip(cursor: &mut StringCursor, n: usize) {
        cursor.position -= n;
        unsafe { cursor.ptr = cursor.ptr.sub(n) };
    }
}

impl StrDrive for &str {
    #[inline]
    fn count(&self) -> usize {
        self.chars().count()
    }

    #[inline]
    fn create_cursor(&self, n: usize) -> StringCursor {
        let mut cursor = StringCursor {
            ptr: self.as_ptr(),
            position: 0,
        };
        Self::skip(&mut cursor, n);
        cursor
    }

    #[inline]
    fn adjust_cursor(&self, cursor: &mut StringCursor, n: usize) {
        if cursor.ptr.is_null() || cursor.position > n {
            *cursor = Self::create_cursor(self, n);
        } else if cursor.position < n {
            Self::skip(cursor, n - cursor.position);
        }
    }

    #[inline]
    fn advance(cursor: &mut StringCursor) -> u32 {
        cursor.position += 1;
        unsafe { next_code_point(&mut cursor.ptr) }
    }

    #[inline]
    fn peek(cursor: &StringCursor) -> u32 {
        let mut ptr = cursor.ptr;
        unsafe { next_code_point(&mut ptr) }
    }

    #[inline]
    fn skip(cursor: &mut StringCursor, n: usize) {
        cursor.position += n;
        for _ in 0..n {
            unsafe { next_code_point(&mut cursor.ptr) };
        }
    }

    #[inline]
    fn back_advance(cursor: &mut StringCursor) -> u32 {
        cursor.position -= 1;
        unsafe { next_code_point_reverse(&mut cursor.ptr) }
    }

    #[inline]
    fn back_peek(cursor: &StringCursor) -> u32 {
        let mut ptr = cursor.ptr;
        unsafe { next_code_point_reverse(&mut ptr) }
    }

    #[inline]
    fn back_skip(cursor: &mut StringCursor, n: usize) {
        cursor.position -= n;
        for _ in 0..n {
            unsafe { next_code_point_reverse(&mut cursor.ptr) };
        }
    }
}

impl StrDrive for &Wtf8 {
    #[inline]
    fn count(&self) -> usize {
        self.code_points().count()
    }

    #[inline]
    fn create_cursor(&self, n: usize) -> StringCursor {
        let mut cursor = StringCursor {
            ptr: self.as_bytes().as_ptr(),
            position: 0,
        };
        Self::skip(&mut cursor, n);
        cursor
    }

    #[inline]
    fn adjust_cursor(&self, cursor: &mut StringCursor, n: usize) {
        if cursor.ptr.is_null() || cursor.position > n {
            *cursor = Self::create_cursor(self, n);
        } else if cursor.position < n {
            Self::skip(cursor, n - cursor.position);
        }
    }

    #[inline]
    fn advance(cursor: &mut StringCursor) -> u32 {
        cursor.position += 1;
        unsafe { next_code_point(&mut cursor.ptr) }
    }

    #[inline]
    fn peek(cursor: &StringCursor) -> u32 {
        let mut ptr = cursor.ptr;
        unsafe { next_code_point(&mut ptr) }
    }

    #[inline]
    fn skip(cursor: &mut StringCursor, n: usize) {
        cursor.position += n;
        for _ in 0..n {
            unsafe { next_code_point(&mut cursor.ptr) };
        }
    }

    #[inline]
    fn back_advance(cursor: &mut StringCursor) -> u32 {
        cursor.position -= 1;
        unsafe { next_code_point_reverse(&mut cursor.ptr) }
    }

    #[inline]
    fn back_peek(cursor: &StringCursor) -> u32 {
        let mut ptr = cursor.ptr;
        unsafe { next_code_point_reverse(&mut ptr) }
    }

    #[inline]
    fn back_skip(cursor: &mut StringCursor, n: usize) {
        cursor.position -= n;
        for _ in 0..n {
            unsafe { next_code_point_reverse(&mut cursor.ptr) };
        }
    }
}

/// Reads the next code point out of a byte iterator (assuming a
/// UTF-8-like encoding).
///
/// # Safety
///
/// `bytes` must produce a valid UTF-8-like (UTF-8 or WTF-8) string
#[inline]
const unsafe fn next_code_point(ptr: &mut *const u8) -> u32 {
    // Decode UTF-8
    let x = unsafe { **ptr };
    *ptr = unsafe { ptr.offset(1) };

    if x < 128 {
        return x as u32;
    }

    // Multibyte case follows
    // Decode from a byte combination out of: [[[x y] z] w]
    // NOTE: Performance is sensitive to the exact formulation here
    let init = utf8_first_byte(x, 2);
    // SAFETY: `bytes` produces an UTF-8-like string,
    // so the iterator must produce a value here.
    let y = unsafe { **ptr };
    *ptr = unsafe { ptr.offset(1) };
    let mut ch = utf8_acc_cont_byte(init, y);
    if x >= 0xE0 {
        // [[x y z] w] case
        // 5th bit in 0xE0 .. 0xEF is always clear, so `init` is still valid
        // SAFETY: `bytes` produces an UTF-8-like string,
        // so the iterator must produce a value here.
        let z = unsafe { **ptr };
        *ptr = unsafe { ptr.offset(1) };
        let y_z = utf8_acc_cont_byte((y & CONT_MASK) as u32, z);
        ch = (init << 12) | y_z;
        if x >= 0xF0 {
            // [x y z w] case
            // use only the lower 3 bits of `init`
            // SAFETY: `bytes` produces an UTF-8-like string,
            // so the iterator must produce a value here.
            let w = unsafe { **ptr };
            *ptr = unsafe { ptr.offset(1) };
            ch = ((init & 7) << 18) | utf8_acc_cont_byte(y_z, w);
        }
    }

    ch
}

/// Reads the last code point out of a byte iterator (assuming a
/// UTF-8-like encoding).
///
/// # Safety
///
/// `bytes` must produce a valid UTF-8-like (UTF-8 or WTF-8) string
#[inline]
const unsafe fn next_code_point_reverse(ptr: &mut *const u8) -> u32 {
    // Decode UTF-8
    *ptr = unsafe { ptr.offset(-1) };
    let w = match unsafe { **ptr } {
        next_byte if next_byte < 128 => return next_byte as u32,
        back_byte => back_byte,
    };

    // Multibyte case follows
    // Decode from a byte combination out of: [x [y [z w]]]
    let mut ch;
    // SAFETY: `bytes` produces an UTF-8-like string,
    // so the iterator must produce a value here.
    *ptr = unsafe { ptr.offset(-1) };
    let z = unsafe { **ptr };
    ch = utf8_first_byte(z, 2);
    if utf8_is_cont_byte(z) {
        // SAFETY: `bytes` produces an UTF-8-like string,
        // so the iterator must produce a value here.
        *ptr = unsafe { ptr.offset(-1) };
        let y = unsafe { **ptr };
        ch = utf8_first_byte(y, 3);
        if utf8_is_cont_byte(y) {
            // SAFETY: `bytes` produces an UTF-8-like string,
            // so the iterator must produce a value here.
            *ptr = unsafe { ptr.offset(-1) };
            let x = unsafe { **ptr };
            ch = utf8_first_byte(x, 4);
            ch = utf8_acc_cont_byte(ch, y);
        }
        ch = utf8_acc_cont_byte(ch, z);
    }
    ch = utf8_acc_cont_byte(ch, w);

    ch
}

/// Returns the initial codepoint accumulator for the first byte.
/// The first byte is special, only want bottom 5 bits for width 2, 4 bits
/// for width 3, and 3 bits for width 4.
#[inline]
const fn utf8_first_byte(byte: u8, width: u32) -> u32 {
    (byte & (0x7F >> width)) as u32
}

/// Returns the value of `ch` updated with continuation byte `byte`.
#[inline]
const fn utf8_acc_cont_byte(ch: u32, byte: u8) -> u32 {
    (ch << 6) | (byte & CONT_MASK) as u32
}

/// Checks whether the byte is a UTF-8 continuation byte (i.e., starts with the
/// bits `10`).
#[inline]
const fn utf8_is_cont_byte(byte: u8) -> bool {
    (byte as i8) < -64
}

/// Mask of the value bits of a continuation byte.
const CONT_MASK: u8 = 0b0011_1111;
