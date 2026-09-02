//! Line-by-line Rust port of PyPy cjkcodecs `_codecs_cn.c` stateless codecs.

use super::mappings_cn::{
    GB2312_DECMAP, GB2312_DECMAP_DATA, GB18030_TO_UNIBMP_RANGES, GB18030EXT_DECMAP,
    GB18030EXT_DECMAP_DATA, GB18030EXT_ENCMAP, GB18030EXT_ENCMAP_DATA, GBCOMMON_ENCMAP,
    GBCOMMON_ENCMAP_DATA, GBKEXT_DECMAP, GBKEXT_DECMAP_DATA, MapIndex, NOCHAR, UNIINV,
};
use super::{DecodeOne, EncodeOne};

fn try_map_decode(index: &[MapIndex; 256], data: &[u16], c1: u8, c2: u8) -> Option<u32> {
    let page = index[c1 as usize];
    if !page.present || c2 < page.bottom || c2 > page.top {
        return None;
    }
    let value = data[page.offset + usize::from(c2 - page.bottom)];
    (value != UNIINV).then_some(u32::from(value))
}

fn try_map_encode(index: &[MapIndex; 256], data: &[u16], c: u32) -> Option<u16> {
    if c > 0xffff {
        return None;
    }
    let page = index[(c >> 8) as usize];
    let low = c as u8;
    if !page.present || low < page.bottom || low > page.top {
        return None;
    }
    let value = data[page.offset + usize::from(low - page.bottom)];
    (value != NOCHAR).then_some(value)
}

fn gbk_decode(c1: u8, c2: u8) -> Option<u32> {
    match (c1, c2) {
        (0xa1, 0xaa) => Some(0x2014),
        (0xa8, 0x44) => Some(0x2015),
        (0xa1, 0xa4) => Some(0x00b7),
        _ => try_map_decode(&GB2312_DECMAP, &GB2312_DECMAP_DATA, c1 ^ 0x80, c2 ^ 0x80)
            .or_else(|| try_map_decode(&GBKEXT_DECMAP, &GBKEXT_DECMAP_DATA, c1, c2)),
    }
}

fn gbk_encode(c: u32) -> Option<u16> {
    match c {
        0x2014 => Some(0xa1aa),
        0x2015 => Some(0xa844),
        0x00b7 => Some(0xa1a4),
        0x30fb => None,
        _ => try_map_encode(&GBCOMMON_ENCMAP, &GBCOMMON_ENCMAP_DATA, c),
    }
}

fn ascii(c: u32) -> Option<EncodeOne> {
    if c >= 0x80 {
        return None;
    }
    let mut output = [0; 8];
    output[0] = c as u8;
    Some(EncodeOne::Bytes(output, 1, 1))
}

fn dbcs(code: u16) -> EncodeOne {
    let mut output = [0; 8];
    output[0] = (code >> 8) as u8 | 0x80;
    output[1] = if code & 0x8000 != 0 {
        code as u8
    } else {
        code as u8 | 0x80
    };
    EncodeOne::Bytes(output, 2, 1)
}

pub(super) fn encode_gb2312(c: u32) -> EncodeOne {
    if let Some(output) = ascii(c) {
        return output;
    }
    let Some(code) = try_map_encode(&GBCOMMON_ENCMAP, &GBCOMMON_ENCMAP_DATA, c) else {
        return EncodeOne::Illegal(1);
    };
    if code & 0x8000 != 0 {
        return EncodeOne::Illegal(1);
    }
    dbcs(code)
}

pub(super) fn decode_gb2312(input: &[u8]) -> DecodeOne {
    let c = input[0];
    if c < 0x80 {
        return DecodeOne::Char(u32::from(c), 1);
    }
    if input.len() < 2 {
        return DecodeOne::Incomplete;
    }
    try_map_decode(
        &GB2312_DECMAP,
        &GB2312_DECMAP_DATA,
        c ^ 0x80,
        input[1] ^ 0x80,
    )
    .map_or(DecodeOne::Illegal(1), |value| DecodeOne::Char(value, 2))
}

pub(super) fn encode_gbk(c: u32) -> EncodeOne {
    if let Some(output) = ascii(c) {
        return output;
    }
    gbk_encode(c).map_or(EncodeOne::Illegal(1), dbcs)
}

pub(super) fn decode_gbk(input: &[u8]) -> DecodeOne {
    let c = input[0];
    if c < 0x80 {
        return DecodeOne::Char(u32::from(c), 1);
    }
    if input.len() < 2 {
        return DecodeOne::Incomplete;
    }
    gbk_decode(c, input[1]).map_or(DecodeOne::Illegal(1), |value| DecodeOne::Char(value, 2))
}

fn four_bytes(mut value: u32, first_base: u8) -> EncodeOne {
    let mut output = [0; 8];
    output[3] = (value % 10) as u8 + 0x30;
    value /= 10;
    output[2] = (value % 126) as u8 + 0x81;
    value /= 126;
    output[1] = (value % 10) as u8 + 0x30;
    value /= 10;
    output[0] = value as u8 + first_base;
    EncodeOne::Bytes(output, 4, 1)
}

pub(super) fn encode_gb18030(c: u32) -> EncodeOne {
    if let Some(output) = ascii(c) {
        return output;
    }
    if c > 0x10ffff || (0xd800..=0xdfff).contains(&c) {
        return EncodeOne::Illegal(1);
    }
    if c >= 0x10000 {
        return four_bytes(c - 0x10000, 0x90);
    }
    if let Some(code) =
        gbk_encode(c).or_else(|| try_map_encode(&GB18030EXT_ENCMAP, &GB18030EXT_ENCMAP_DATA, c))
    {
        return dbcs(code);
    }
    for range in &GB18030_TO_UNIBMP_RANGES {
        if range.first == 0 {
            break;
        }
        if range.first <= c && c <= range.last {
            return four_bytes(c - range.first + range.base, 0x81);
        }
    }
    EncodeOne::Illegal(1)
}

pub(super) fn decode_gb18030(input: &[u8]) -> DecodeOne {
    let c = input[0];
    if c < 0x80 {
        return DecodeOne::Char(u32::from(c), 1);
    }
    if input.len() < 2 {
        return DecodeOne::Incomplete;
    }
    let c2 = input[1];
    if (0x30..=0x39).contains(&c2) {
        if input.len() < 4 {
            return DecodeOne::Incomplete;
        }
        let c3 = input[2];
        let c4 = input[3];
        if !(0x81..=0xfe).contains(&c)
            || !(0x81..=0xfe).contains(&c3)
            || !(0x30..=0x39).contains(&c4)
        {
            return DecodeOne::Illegal(1);
        }
        let lead = c - 0x81;
        let second = c2 - 0x30;
        let third = c3 - 0x81;
        let fourth = c4 - 0x30;
        if lead < 4 {
            let sequence = (u32::from(lead) * 10 + u32::from(second)) * 1260
                + u32::from(third) * 10
                + u32::from(fourth);
            if sequence < 39420 {
                let mut range = &GB18030_TO_UNIBMP_RANGES[0];
                for next in &GB18030_TO_UNIBMP_RANGES[1..] {
                    if sequence < next.base {
                        break;
                    }
                    range = next;
                }
                return DecodeOne::Char(range.first - range.base + sequence, 4);
            }
        } else if lead >= 15 {
            let value = 0x10000
                + ((u32::from(lead) - 15) * 10 + u32::from(second)) * 1260
                + u32::from(third) * 10
                + u32::from(fourth);
            if value <= 0x10ffff {
                return DecodeOne::Char(value, 4);
            }
        }
        return DecodeOne::Illegal(1);
    }

    gbk_decode(c, c2)
        .or_else(|| try_map_decode(&GB18030EXT_DECMAP, &GB18030EXT_DECMAP_DATA, c, c2))
        .map_or(DecodeOne::Illegal(1), |value| DecodeOne::Char(value, 2))
}

pub(super) fn encode_hz(c: u32, state: &mut [u8; 8]) -> EncodeOne {
    let mut output = [0; 8];
    if c < 0x80 {
        let mut length = 0;
        if state[0] != 0 {
            output[0] = b'~';
            output[1] = b'}';
            length = 2;
            state[0] = 0;
        }
        output[length] = c as u8;
        length += 1;
        if c == u32::from(b'~') {
            output[length] = b'~';
            length += 1;
        }
        return EncodeOne::Bytes(output, length, 1);
    }

    let Some(code) = try_map_encode(&GBCOMMON_ENCMAP, &GBCOMMON_ENCMAP_DATA, c) else {
        return EncodeOne::Illegal(1);
    };
    if code & 0x8000 != 0 {
        return EncodeOne::Illegal(1);
    }
    if state[0] == 0 {
        output[0] = b'~';
        output[1] = b'{';
        output[2] = (code >> 8) as u8;
        output[3] = code as u8;
        state[0] = 1;
        EncodeOne::Bytes(output, 4, 1)
    } else {
        output[0] = (code >> 8) as u8;
        output[1] = code as u8;
        EncodeOne::Bytes(output, 2, 1)
    }
}

pub(super) fn reset_hz(state: &mut [u8; 8]) -> Option<([u8; 8], usize)> {
    if state[0] == 0 {
        return None;
    }
    let mut output = [0; 8];
    output[0] = b'~';
    output[1] = b'}';
    state[0] = 0;
    Some((output, 2))
}

pub(super) fn reset_decode_hz(state: &mut [u8; 8]) {
    state[0] = 0;
}

pub(super) fn decode_hz(input: &[u8], state: &mut [u8; 8]) -> DecodeOne {
    let c = input[0];
    if c == b'~' {
        if input.len() < 2 {
            return DecodeOne::Incomplete;
        }
        let c2 = input[1];
        if c2 == b'~' && state[0] == 0 {
            return DecodeOne::Char(u32::from(b'~'), 2);
        }
        if c2 == b'{' && state[0] == 0 {
            state[0] = 1;
        } else if c2 == b'\n' && state[0] == 0 {
            // line continuation
        } else if c2 == b'}' && state[0] == 1 {
            state[0] = 0;
        } else {
            return DecodeOne::Illegal(1);
        }
        return DecodeOne::Skip(2);
    }
    if c & 0x80 != 0 {
        return DecodeOne::Illegal(1);
    }
    if state[0] == 0 {
        return DecodeOne::Char(u32::from(c), 1);
    }
    if input.len() < 2 {
        return DecodeOne::Incomplete;
    }
    try_map_decode(&GB2312_DECMAP, &GB2312_DECMAP_DATA, c, input[1])
        .map_or(DecodeOne::Illegal(1), |value| DecodeOne::Char(value, 2))
}

#[cfg(test)]
mod tests {
    use super::super::{Codec, DecodeOne, EncodeOne, decode_one, encode_one};

    fn assert_vector(codec: Codec, value: u32, expected: &[u8]) {
        let EncodeOne::Bytes(bytes, len, 1) = encode_one(codec, &[value], true, &mut [0; 8]) else {
            panic!("U+{value:04X} is not encodable");
        };
        assert_eq!(&bytes[..len], expected);
        assert!(matches!(
            decode_one(codec, expected, &mut [0; 8]),
            DecodeOne::Char(decoded, consumed)
                if decoded == value && consumed == expected.len()
        ));
    }

    #[test]
    fn pypy_cn_oracle_vectors() {
        assert_vector(Codec::Gb2312, 0x30fb, &[0xa1, 0xa4]);
        assert_vector(Codec::Gbk, 0x00b7, &[0xa1, 0xa4]);
        assert_vector(Codec::Gbk, 0x2014, &[0xa1, 0xaa]);
        assert_vector(Codec::Gbk, 0x2015, &[0xa8, 0x44]);
        assert_vector(Codec::Gb18030, 0x0080, &[0x81, 0x30, 0x81, 0x30]);
        assert_vector(Codec::Gb18030, 0x30fb, &[0x81, 0x39, 0xa7, 0x39]);
        assert_vector(Codec::Gb18030, 0x10000, &[0x90, 0x30, 0x81, 0x30]);
        assert_vector(Codec::Gb18030, 0x1f600, &[0x94, 0x39, 0xfc, 0x36]);
    }

    #[test]
    fn pypy_hz_shift_state_and_reset() {
        let mut state = [0; 8];
        assert!(matches!(
            encode_one(Codec::Hz, &['聊' as u32], false, &mut state),
            EncodeOne::Bytes(bytes, 4, 1) if bytes[..4] == *b"~{AD"
        ));
        assert_eq!(state[0], 1);
        assert!(matches!(
            encode_one(Codec::Hz, &['聊' as u32], false, &mut state),
            EncodeOne::Bytes(bytes, 2, 1) if bytes[..2] == *b"AD"
        ));
        let (reset, length) = super::super::encode_reset(Codec::Hz, &mut state).unwrap();
        assert_eq!(&reset[..length], b"~}");
        assert_eq!(state[0], 0);

        assert!(matches!(
            decode_one(Codec::Hz, b"~{", &mut state),
            DecodeOne::Skip(2)
        ));
        assert_eq!(state[0], 1);
        assert!(matches!(
            decode_one(Codec::Hz, b"AD", &mut state),
            DecodeOne::Char(value, 2) if value == '聊' as u32
        ));
    }
}
