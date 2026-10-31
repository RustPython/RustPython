//! Line-by-line Rust port of PyPy cjkcodecs `_codecs_tw.c`.

use super::mappings_tw::{
    BIG5_DECMAP, BIG5_DECMAP_DATA, BIG5_ENCMAP, BIG5_ENCMAP_DATA, CP950EXT_DECMAP,
    CP950EXT_DECMAP_DATA, CP950EXT_ENCMAP, CP950EXT_ENCMAP_DATA, MapIndex, NOCHAR, UNIINV,
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

fn encode_ascii(c: u32) -> Option<EncodeOne> {
    if c >= 0x80 {
        return None;
    }
    let mut output = [0; 8];
    output[0] = c as u8;
    Some(EncodeOne::Bytes(output, 1, 1))
}

fn encode_dbcs(code: u16) -> EncodeOne {
    let mut output = [0; 8];
    output[0] = (code >> 8) as u8;
    output[1] = code as u8;
    EncodeOne::Bytes(output, 2, 1)
}

pub(super) fn encode_big5(c: u32) -> EncodeOne {
    if let Some(output) = encode_ascii(c) {
        return output;
    }
    try_map_encode(&BIG5_ENCMAP, &BIG5_ENCMAP_DATA, c).map_or(EncodeOne::Illegal(1), encode_dbcs)
}

pub(super) fn decode_big5(input: &[u8]) -> DecodeOne {
    let c = input[0];
    if c < 0x80 {
        return DecodeOne::Char(u32::from(c), 1);
    }
    if input.len() < 2 {
        return DecodeOne::Incomplete;
    }
    try_map_decode(&BIG5_DECMAP, &BIG5_DECMAP_DATA, c, input[1])
        // The checked-in `_codecs_tw.c::DECODER(big5)` returns two here, but
        // the real PyPy codec oracle exposes a one-byte error span (as does
        // 3.14).  Preserve that observable PyPy result at the Rust boundary.
        .map_or(DecodeOne::Illegal(1), |value| DecodeOne::Char(value, 2))
}

pub(super) fn encode_cp950(c: u32) -> EncodeOne {
    if let Some(output) = encode_ascii(c) {
        return output;
    }
    try_map_encode(&CP950EXT_ENCMAP, &CP950EXT_ENCMAP_DATA, c)
        .or_else(|| try_map_encode(&BIG5_ENCMAP, &BIG5_ENCMAP_DATA, c))
        .map_or(EncodeOne::Illegal(1), encode_dbcs)
}

pub(super) fn decode_cp950(input: &[u8]) -> DecodeOne {
    let c = input[0];
    if c < 0x80 {
        return DecodeOne::Char(u32::from(c), 1);
    }
    if input.len() < 2 {
        return DecodeOne::Incomplete;
    }
    try_map_decode(&CP950EXT_DECMAP, &CP950EXT_DECMAP_DATA, c, input[1])
        .or_else(|| try_map_decode(&BIG5_DECMAP, &BIG5_DECMAP_DATA, c, input[1]))
        .map_or(DecodeOne::Illegal(1), |value| DecodeOne::Char(value, 2))
}

#[cfg(test)]
mod tests {
    use super::super::{Codec, decode_one, encode_one};
    use super::{DecodeOne, EncodeOne};

    #[test]
    fn pypy_tw_oracle_vectors() {
        for (codec, c, expected) in [
            (Codec::Big5, '漢', [0xba, 0x7e]),
            (Codec::Cp950, '漢', [0xba, 0x7e]),
            (Codec::Cp950, '€', [0xa3, 0xe1]),
        ] {
            let points = [c as u32];
            let EncodeOne::Bytes(bytes, 2, 1) = encode_one(codec, &points, true, &mut [0; 8])
            else {
                panic!("{c:?} is not encodable");
            };
            assert_eq!(&bytes[..2], expected);
            assert!(matches!(
                decode_one(codec, &expected, &mut [0; 8]),
                DecodeOne::Char(value, 2) if value == c as u32
            ));
        }
    }

    #[test]
    fn malformed_candidates_reject_the_lead_byte() {
        assert!(matches!(
            decode_one(Codec::Big5, &[0x81, 0x00], &mut [0; 8]),
            DecodeOne::Illegal(1)
        ));
        assert!(matches!(
            decode_one(Codec::Cp950, &[0x81, 0x00], &mut [0; 8]),
            DecodeOne::Illegal(1)
        ));
    }
}
