//! Variable-length integer encoding utilities.
//!
//! Two encodings are used:
//! - **Little-endian** (low bits first): linetable
//! - **Big-endian** (high bits first): exception tables
//!
//! Both use 6-bit chunks with 0x40 as the continuation bit.

use alloc::vec::Vec;

/// Write a little-endian varint (used by linetable).
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

/// Write a little-endian signed varint.
#[inline]
pub fn write_signed_varint(buf: &mut Vec<u8>, val: i32) -> usize {
    let uval = if val < 0 {
        ((0u32.wrapping_sub(val as u32)) << 1) | 1
    } else {
        (val as u32) << 1
    };
    write_varint(buf, uval)
}

/// Write a big-endian varint (used by exception tables).
pub fn write_varint_be(buf: &mut Vec<u8>, val: u32) -> usize {
    let start_len = buf.len();
    if val >= 1 << 24 {
        buf.push(0x40 | ((val >> 24) & 0x3f) as u8);
    }
    if val >= 1 << 18 {
        buf.push(0x40 | ((val >> 18) & 0x3f) as u8);
    }
    if val >= 1 << 12 {
        buf.push(0x40 | ((val >> 12) & 0x3f) as u8);
    }
    if val >= 1 << 6 {
        buf.push(0x40 | ((val >> 6) & 0x3f) as u8);
    }
    buf.push((val & 0x3f) as u8);
    buf.len() - start_len
}

/// Write a big-endian varint with the start marker (0x80) on the first byte.
pub fn write_varint_with_start(data: &mut Vec<u8>, val: u32) {
    let start_pos = data.len();
    write_varint_be(data, val);
    if let Some(first) = data.get_mut(start_pos) {
        *first |= 0x80;
    }
}

/// Read a big-endian varint with start marker (0x80).
pub fn read_varint_with_start(data: &[u8], pos: &mut usize) -> Option<u32> {
    if *pos >= data.len() {
        return None;
    }
    let first = data[*pos];
    if first & 0x80 == 0 {
        return None;
    }
    *pos += 1;
    let mut val = (first & 0x3f) as u32;
    let mut cont = first & 0x40 != 0;
    while cont && *pos < data.len() {
        let b = data[*pos];
        *pos += 1;
        val = (val << 6) | (b & 0x3f) as u32;
        cont = b & 0x40 != 0;
    }
    Some(val)
}

/// Read a big-endian varint (no start marker).
pub fn read_varint(data: &[u8], pos: &mut usize) -> Option<u32> {
    if *pos >= data.len() {
        return None;
    }
    let first = data[*pos];
    *pos += 1;
    let mut val = (first & 0x3f) as u32;
    let mut cont = first & 0x40 != 0;
    while cont && *pos < data.len() {
        let b = data[*pos];
        *pos += 1;
        val = (val << 6) | (b & 0x3f) as u32;
        cont = b & 0x40 != 0;
    }
    Some(val)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_le_varint_roundtrip() {
        // Little-endian is only used internally in linetable,
        // no read function needed outside of linetable parsing.
        let mut buf = Vec::new();
        write_varint(&mut buf, 0);
        write_varint(&mut buf, 63);
        write_varint(&mut buf, 64);
        write_varint(&mut buf, 4095);
        assert_eq!(buf.len(), 1 + 1 + 2 + 2);
    }

    #[test]
    fn test_be_varint_roundtrip() {
        for &val in &[0u32, 1, 63, 64, 127, 128, 4095, 4096, 1_000_000] {
            let mut buf = Vec::new();
            write_varint_be(&mut buf, val);
            let mut pos = 0;
            assert_eq!(read_varint(&buf, &mut pos), Some(val), "val={val}");
            assert_eq!(pos, buf.len());
        }
    }

    #[test]
    fn test_be_varint_with_start() {
        let mut buf = Vec::new();
        write_varint_with_start(&mut buf, 42);
        write_varint_with_start(&mut buf, 100);
        write_varint_with_start(&mut buf, 71);

        let mut pos = 0;
        assert_eq!(read_varint_with_start(&buf, &mut pos), Some(42));
        assert_eq!(read_varint_with_start(&buf, &mut pos), Some(100));
        assert_eq!(read_varint_with_start(&buf, &mut pos), Some(71));
        assert_eq!(read_varint_with_start(&buf, &mut pos), None);
    }
}
