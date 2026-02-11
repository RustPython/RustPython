//! Variable-length integer encoding utilities.
//!
//! Uses 6-bit chunks with a continuation bit (0x40) to encode integers.
//! Used for exception tables and line number tables.

use alloc::vec::Vec;

/// Write a variable-length unsigned integer using 6-bit chunks.
/// Returns the number of bytes written.
#[inline]
pub fn write_varint(buf: &mut Vec<u8>, mut val: u32) -> usize {
    let start_len = buf.len();
    while val >= 64 {
        buf.push(0x40 | (val & 0x3f) as u8);
        val >>= 6;
    }
    buf.push(val as u8);
    buf.len() - start_len
}

/// Write a variable-length signed integer.
/// Returns the number of bytes written.
#[inline]
pub fn write_signed_varint(buf: &mut Vec<u8>, val: i32) -> usize {
    let uval = if val < 0 {
        // (0 - val as u32) handles INT_MIN correctly
        ((0u32.wrapping_sub(val as u32)) << 1) | 1
    } else {
        (val as u32) << 1
    };
    write_varint(buf, uval)
}

/// Write a variable-length unsigned integer with a start marker (0x80 bit).
/// Used for exception table entries where each entry starts with the marker.
pub fn write_varint_with_start(data: &mut Vec<u8>, val: u32) {
    let start_pos = data.len();
    write_varint(data, val);
    // Set start bit on first byte
    if let Some(first) = data.get_mut(start_pos) {
        *first |= 0x80;
    }
}

/// Read a variable-length unsigned integer that starts with a start marker (0x80 bit).
/// Returns None if not at a valid start byte or end of data.
pub fn read_varint_with_start(data: &[u8], pos: &mut usize) -> Option<u32> {
    if *pos >= data.len() {
        return None;
    }
    let first = data[*pos];
    if first & 0x80 == 0 {
        return None; // Not a start byte
    }
    *pos += 1;

    let mut val = (first & 0x3f) as u32;
    let mut shift = 6;
    let mut has_continuation = first & 0x40 != 0;

    while has_continuation && *pos < data.len() {
        let byte = data[*pos];
        if byte & 0x80 != 0 {
            break; // Next entry start
        }
        *pos += 1;
        val |= ((byte & 0x3f) as u32) << shift;
        shift += 6;
        has_continuation = byte & 0x40 != 0;
    }
    Some(val)
}

/// Read a variable-length unsigned integer.
/// Returns None if end of data or malformed.
pub fn read_varint(data: &[u8], pos: &mut usize) -> Option<u32> {
    if *pos >= data.len() {
        return None;
    }

    let mut val = 0u32;
    let mut shift = 0;

    loop {
        if *pos >= data.len() {
            return None;
        }
        let byte = data[*pos];
        if byte & 0x80 != 0 && shift > 0 {
            break; // Next entry start
        }
        *pos += 1;
        val |= ((byte & 0x3f) as u32) << shift;
        shift += 6;
        if byte & 0x40 == 0 {
            break;
        }
    }
    Some(val)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_write_read_varint() {
        let mut buf = Vec::new();
        write_varint(&mut buf, 0);
        write_varint(&mut buf, 63);
        write_varint(&mut buf, 64);
        write_varint(&mut buf, 4095);

        // Values: 0, 63, 64, 4095
        assert_eq!(buf.len(), 1 + 1 + 2 + 2);
    }

    #[test]
    fn test_write_read_signed_varint() {
        let mut buf = Vec::new();
        write_signed_varint(&mut buf, 0);
        write_signed_varint(&mut buf, 1);
        write_signed_varint(&mut buf, -1);
        write_signed_varint(&mut buf, i32::MIN);

        assert!(!buf.is_empty());
    }

    #[test]
    fn test_varint_with_start() {
        let mut buf = Vec::new();
        write_varint_with_start(&mut buf, 42);
        write_varint_with_start(&mut buf, 100);

        let mut pos = 0;
        assert_eq!(read_varint_with_start(&buf, &mut pos), Some(42));
        assert_eq!(read_varint_with_start(&buf, &mut pos), Some(100));
        assert_eq!(read_varint_with_start(&buf, &mut pos), None);
    }
}
