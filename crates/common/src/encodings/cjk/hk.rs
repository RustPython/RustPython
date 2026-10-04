//! Line-by-line Rust port of PyPy cjkcodecs `_codecs_hk.c`.

use super::mappings_hk::{
    BIG5HKSCS_BMP_ENCMAP, BIG5HKSCS_BMP_ENCMAP_DATA, BIG5HKSCS_DECMAP, BIG5HKSCS_DECMAP_DATA,
    BIG5HKSCS_NONBMP_ENCMAP, BIG5HKSCS_NONBMP_ENCMAP_DATA, BIG5HKSCS_PHINT_0_DATA,
    BIG5HKSCS_PHINT_12130_DATA, BIG5HKSCS_PHINT_21924_DATA, MULTIC,
};
use super::mappings_tw::{BIG5_DECMAP, BIG5_DECMAP_DATA, BIG5_ENCMAP, BIG5_ENCMAP_DATA};
use super::{DecodeOne, EncodeOne};

const PAIR_ENCODE_TABLE: [u16; 4] = [0x8862, 0x8864, 0x88a3, 0x88a5];

fn try_map_decode(
    index: &[super::mappings_hk::MapIndex; 256],
    data: &[u16],
    c1: u8,
    c2: u8,
) -> Option<u32> {
    let page = index[c1 as usize];
    if !page.present || c2 < page.bottom || c2 > page.top {
        return None;
    }
    let value = data[page.offset + usize::from(c2 - page.bottom)];
    (value != super::mappings_hk::UNIINV).then_some(u32::from(value))
}

fn try_map_big5_decode(c1: u8, c2: u8) -> Option<u32> {
    let page = BIG5_DECMAP[c1 as usize];
    if !page.present || c2 < page.bottom || c2 > page.top {
        return None;
    }
    let value = BIG5_DECMAP_DATA[page.offset + usize::from(c2 - page.bottom)];
    (value != super::mappings_tw::UNIINV).then_some(u32::from(value))
}

fn try_map_encode(
    index: &[super::mappings_hk::MapIndex; 256],
    data: &[u16],
    c: u32,
) -> Option<u16> {
    let page = index[(c >> 8) as usize];
    let low = c as u8;
    if !page.present || low < page.bottom || low > page.top {
        return None;
    }
    let value = data[page.offset + usize::from(low - page.bottom)];
    (value != super::mappings_hk::NOCHAR).then_some(value)
}

fn try_map_big5_encode(c: u32) -> Option<u16> {
    if c > 0xffff {
        return None;
    }
    let page = BIG5_ENCMAP[(c >> 8) as usize];
    let low = c as u8;
    if !page.present || low < page.bottom || low > page.top {
        return None;
    }
    let value = BIG5_ENCMAP_DATA[page.offset + usize::from(low - page.bottom)];
    (value != super::mappings_tw::NOCHAR).then_some(value)
}

fn encoded(code: u16, consumed: usize) -> EncodeOne {
    let mut output = [0; 8];
    output[0] = (code >> 8) as u8;
    output[1] = code as u8;
    EncodeOne::Bytes(output, 2, consumed)
}

pub(super) fn encode_big5hkscs(input: &[u32], final_input: bool) -> EncodeOne {
    let c = input[0];
    if c < 0x80 {
        let mut output = [0; 8];
        output[0] = c as u8;
        return EncodeOne::Bytes(output, 1, 1);
    }

    let code = if c < 0x10000 {
        if let Some(mut code) = try_map_encode(&BIG5HKSCS_BMP_ENCMAP, &BIG5HKSCS_BMP_ENCMAP_DATA, c)
        {
            if code == MULTIC {
                if input.len() >= 2 && c & 0xffdf == 0x00ca && input[1] & 0xfff7 == 0x0304 {
                    code = PAIR_ENCODE_TABLE[((c >> 4 | input[1] >> 3) & 3) as usize];
                    return encoded(code, 2);
                }
                if input.len() < 2 && !final_input {
                    return EncodeOne::Incomplete;
                }
                code = if c == 0x00ca { 0x8866 } else { 0x88a7 };
            }
            code
        } else if let Some(code) = try_map_big5_encode(c) {
            code
        } else {
            return EncodeOne::Illegal(1);
        }
    } else if c < 0x20000 {
        return EncodeOne::Illegal(1);
    } else if c < 0x30000 {
        let Some(code) = try_map_encode(
            &BIG5HKSCS_NONBMP_ENCMAP,
            &BIG5HKSCS_NONBMP_ENCMAP_DATA,
            c & 0xffff,
        ) else {
            return EncodeOne::Illegal(1);
        };
        code
    } else {
        return EncodeOne::Illegal(1);
    };
    encoded(code, 1)
}

fn bh2s(c1: u8, c2: u8) -> usize {
    usize::from(c1 - 0x87) * (0xfe - 0x40 + 1) + usize::from(c2 - 0x40)
}

pub(super) fn decode_big5hkscs(input: &[u8]) -> DecodeOne {
    let c = input[0];
    if c < 0x80 {
        return DecodeOne::Char(u32::from(c), 1);
    }
    if input.len() < 2 {
        return DecodeOne::Incomplete;
    }
    let c2 = input[1];

    let force_hkscs = (0xc6..=0xc8).contains(&c) && (c >= 0xc7 || c2 >= 0xa1);
    if !force_hkscs {
        if let Some(value) = try_map_big5_decode(c, c2) {
            return DecodeOne::Char(value, 2);
        }
    }

    if let Some(decoded) = try_map_decode(&BIG5HKSCS_DECMAP, &BIG5HKSCS_DECMAP_DATA, c, c2) {
        // `_codecs_hk.c::DECODER(big5hkscs)` uses the three bit-vector hints
        // to distinguish BMP values from table entries whose high plane is 2.
        if c < 0x87 || c2 < 0x40 {
            debug_assert!(false, "mapping violates PyPy BIG5HKSCS bounds");
            return DecodeOne::Illegal(1);
        }
        let s = bh2s(c, c2);
        let (hints, relative) = if s <= bh2s(0xa0, 0xfe) {
            (&BIG5HKSCS_PHINT_0_DATA[..], s)
        } else if (bh2s(0xc6, 0xa1)..=bh2s(0xc8, 0xfe)).contains(&s) {
            (&BIG5HKSCS_PHINT_12130_DATA[..], s - bh2s(0xc6, 0xa1))
        } else if (bh2s(0xf9, 0xd6)..=bh2s(0xfe, 0xfe)).contains(&s) {
            (&BIG5HKSCS_PHINT_21924_DATA[..], s - bh2s(0xf9, 0xd6))
        } else {
            debug_assert!(false, "mapping violates PyPy BIG5HKSCS hint ranges");
            return DecodeOne::Illegal(1);
        };
        let value = if hints[relative >> 3] & (1 << (relative & 7)) != 0 {
            decoded | 0x20000
        } else {
            decoded
        };
        return DecodeOne::Char(value, 2);
    }

    match (u16::from(c) << 8) | u16::from(c2) {
        0x8862 => DecodeOne::Pair(0x00ca, 0x0304, 2),
        0x8864 => DecodeOne::Pair(0x00ca, 0x030c, 2),
        0x88a3 => DecodeOne::Pair(0x00ea, 0x0304, 2),
        0x88a5 => DecodeOne::Pair(0x00ea, 0x030c, 2),
        _ => DecodeOne::Illegal(1),
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Codec, DecodeOne, EncodeOne, decode_one, encode_one};

    #[test]
    fn pypy_hk_pair_oracle_vectors() {
        for (points, expected) in [
            (&[0x00ca, 0x0304][..], [0x88, 0x62]),
            (&[0x00ca, 0x030c][..], [0x88, 0x64]),
            (&[0x00ea, 0x0304][..], [0x88, 0xa3]),
            (&[0x00ea, 0x030c][..], [0x88, 0xa5]),
        ] {
            let EncodeOne::Bytes(bytes, 2, 2) =
                encode_one(Codec::Big5Hkscs, points, true, &mut [0; 8])
            else {
                panic!("pair is not encodable");
            };
            assert_eq!(&bytes[..2], expected);
            assert!(matches!(
                decode_one(Codec::Big5Hkscs, &expected, &mut [0; 8]),
                DecodeOne::Pair(first, second, 2)
                    if first == points[0] && second == points[1]
            ));
        }
    }

    #[test]
    fn pair_prefix_is_incrementally_incomplete() {
        assert!(matches!(
            encode_one(Codec::Big5Hkscs, &[0x00ca], false, &mut [0; 8]),
            EncodeOne::Incomplete
        ));
        assert!(matches!(
            encode_one(Codec::Big5Hkscs, &[0x00ca], true, &mut [0; 8]),
            EncodeOne::Bytes(bytes, 2, 1) if bytes[..2] == [0x88, 0x66]
        ));
    }
}
