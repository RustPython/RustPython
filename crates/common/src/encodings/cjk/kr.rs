//! Line-by-line Rust port of PyPy cjkcodecs `_codecs_kr.c`.

use super::mappings_kr::{
    CP949_ENCMAP, CP949EXT_DECMAP, CP949EXT_DECMAP_DATA, KSX1001_DECMAP, KSX1001_DECMAP_DATA,
    MapIndex, NOCHAR, UNIINV,
};
use super::{DecodeOne, EncodeOne};

const EUCKR_JAMO_FIRSTBYTE: u8 = 0xa4;
const EUCKR_JAMO_FILLER: u8 = 0xd4;

const U2CGK_CHOSEONG: [u8; 19] = [
    0xa1, 0xa2, 0xa4, 0xa7, 0xa8, 0xa9, 0xb1, 0xb2, 0xb3, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xbb,
    0xbc, 0xbd, 0xbe,
];
const U2CGK_JUNGSEONG: [u8; 21] = [
    0xbf, 0xc0, 0xc1, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xcb, 0xcc, 0xcd, 0xce,
    0xcf, 0xd0, 0xd1, 0xd2, 0xd3,
];
const U2CGK_JONGSEONG: [u8; 28] = [
    0xd4, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa9, 0xaa, 0xab, 0xac, 0xad, 0xae, 0xaf, 0xb0,
    0xb1, 0xb2, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xba, 0xbb, 0xbc, 0xbd, 0xbe,
];

const NONE_CGK: u8 = 127;
const CGK2U_CHOSEONG: [u8; 30] = [
    0, 1, NONE_CGK, 2, NONE_CGK, NONE_CGK, 3, 4, 5, NONE_CGK, NONE_CGK, NONE_CGK, NONE_CGK,
    NONE_CGK, NONE_CGK, NONE_CGK, 6, 7, 8, NONE_CGK, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18,
];
const CGK2U_JONGSEONG: [u8; 30] = [
    1, 2, 3, 4, 5, 6, 7, NONE_CGK, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, NONE_CGK, 18, 19, 20, 21,
    22, NONE_CGK, 23, 24, 25, 26, 27,
];

const U2JOHABIDX_CHOSEONG: [u8; 32] = [
    0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10, 0x11,
    0x12, 0x13, 0x14, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];
const U2JOHABIDX_JUNGSEONG: [u8; 32] = [
    0x03, 0x04, 0x05, 0x06, 0x07, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x12, 0x13, 0x14, 0x15, 0x16,
    0x17, 0x1a, 0x1b, 0x1c, 0x1d, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];
const U2JOHABIDX_JONGSEONG: [u8; 32] = [
    0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10,
    0x11, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0, 0, 0, 0,
];
const U2JOHABJAMO: [u16; 51] = [
    0x8841, 0x8c41, 0x8444, 0x9041, 0x8446, 0x8447, 0x9441, 0x9841, 0x9c41, 0x844a, 0x844b, 0x844c,
    0x844d, 0x844e, 0x844f, 0x8450, 0xa041, 0xa441, 0xa841, 0x8454, 0xac41, 0xb041, 0xb441, 0xb841,
    0xbc41, 0xc041, 0xc441, 0xc841, 0xcc41, 0xd041, 0x8461, 0x8481, 0x84a1, 0x84c1, 0x84e1, 0x8541,
    0x8561, 0x8581, 0x85a1, 0x85c1, 0x85e1, 0x8641, 0x8661, 0x8681, 0x86a1, 0x86c1, 0x86e1, 0x8741,
    0x8761, 0x8781, 0x87a1,
];

const FILL: u8 = 0xfd;
const NONE: u8 = 0xff;
const JOHABIDX_CHOSEONG: [u8; 32] = [
    NONE, FILL, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
    0x0e, 0x0f, 0x10, 0x11, 0x12, NONE, NONE, NONE, NONE, NONE, NONE, NONE, NONE, NONE, NONE, NONE,
];
const JOHABIDX_JUNGSEONG: [u8; 32] = [
    NONE, NONE, FILL, 0x00, 0x01, 0x02, 0x03, 0x04, NONE, NONE, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a,
    NONE, NONE, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10, NONE, NONE, 0x11, 0x12, 0x13, 0x14, NONE, NONE,
];
const JOHABIDX_JONGSEONG: [u8; 32] = [
    NONE, FILL, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
    0x0f, 0x10, NONE, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, NONE, NONE,
];
const JOHABJAMO_CHOSEONG: [u8; 32] = [
    NONE, FILL, 0x31, 0x32, 0x34, 0x37, 0x38, 0x39, 0x41, 0x42, 0x43, 0x45, 0x46, 0x47, 0x48, 0x49,
    0x4a, 0x4b, 0x4c, 0x4d, 0x4e, NONE, NONE, NONE, NONE, NONE, NONE, NONE, NONE, NONE, NONE, NONE,
];
const JOHABJAMO_JUNGSEONG: [u8; 32] = [
    NONE, NONE, FILL, 0x4f, 0x50, 0x51, 0x52, 0x53, NONE, NONE, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59,
    NONE, NONE, 0x5a, 0x5b, 0x5c, 0x5d, 0x5e, 0x5f, NONE, NONE, 0x60, 0x61, 0x62, 0x63, NONE, NONE,
];
const JOHABJAMO_JONGSEONG: [u8; 32] = [
    NONE, FILL, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x39, 0x3a, 0x3b, 0x3c, 0x3d, 0x3e, 0x3f,
    0x40, 0x41, NONE, 0x42, 0x44, 0x45, 0x46, 0x47, 0x48, 0x4a, 0x4b, 0x4c, 0x4d, 0x4e, NONE, NONE,
];

fn try_map_decode(index: &[MapIndex; 256], data: &[u16], c1: u8, c2: u8) -> Option<u32> {
    let page = index[c1 as usize];
    if !page.present || c2 < page.bottom || c2 > page.top {
        return None;
    }
    let value = data[page.offset + usize::from(c2 - page.bottom)];
    (value != UNIINV).then_some(u32::from(value))
}

fn try_map_encode(c: u32) -> Option<u16> {
    if c > 0xffff {
        return None;
    }
    let page = CP949_ENCMAP[(c >> 8) as usize];
    let low = c as u8;
    if !page.present || low < page.bottom || low > page.top {
        return None;
    }
    let value = super::mappings_kr::CP949_ENCMAP_DATA[page.offset + usize::from(low - page.bottom)];
    (value != NOCHAR).then_some(value)
}

fn bytes2(c1: u8, c2: u8) -> EncodeOne {
    let mut output = [0; 8];
    output[0] = c1;
    output[1] = c2;
    EncodeOne::Bytes(output, 2, 1)
}

pub(super) fn encode_euc_kr(c: u32) -> EncodeOne {
    if c < 0x80 {
        let mut output = [0; 8];
        output[0] = c as u8;
        return EncodeOne::Bytes(output, 1, 1);
    }
    let Some(code) = try_map_encode(c) else {
        return EncodeOne::Illegal(1);
    };
    if code & 0x8000 == 0 {
        return bytes2((code >> 8) as u8 | 0x80, code as u8 | 0x80);
    }

    // `_codecs_kr.c::ENCODER(euc_kr)`: KS X 1001:1998 Annex 3 make-up
    // sequence, retaining PyPy's syllable-composition order.
    debug_assert!((0xac00..=0xd7a3).contains(&c));
    let syllable = c - 0xac00;
    let output = [
        EUCKR_JAMO_FIRSTBYTE,
        EUCKR_JAMO_FILLER,
        EUCKR_JAMO_FIRSTBYTE,
        U2CGK_CHOSEONG[(syllable / 588) as usize],
        EUCKR_JAMO_FIRSTBYTE,
        U2CGK_JUNGSEONG[((syllable / 28) % 21) as usize],
        EUCKR_JAMO_FIRSTBYTE,
        U2CGK_JONGSEONG[(syllable % 28) as usize],
    ];
    EncodeOne::Bytes(output, 8, 1)
}

pub(super) fn decode_euc_kr(input: &[u8]) -> DecodeOne {
    let c = input[0];
    if c < 0x80 {
        return DecodeOne::Char(u32::from(c), 1);
    }
    if input.len() < 2 {
        return DecodeOne::Incomplete;
    }
    if c == EUCKR_JAMO_FIRSTBYTE && input[1] == EUCKR_JAMO_FILLER {
        if input.len() < 8 {
            return DecodeOne::Incomplete;
        }
        if input[2] != EUCKR_JAMO_FIRSTBYTE
            || input[4] != EUCKR_JAMO_FIRSTBYTE
            || input[6] != EUCKR_JAMO_FIRSTBYTE
        {
            return DecodeOne::Illegal(1);
        }
        let cho = if (0xa1..=0xbe).contains(&input[3]) {
            CGK2U_CHOSEONG[(input[3] - 0xa1) as usize]
        } else {
            NONE_CGK
        };
        let jung = if (0xbf..=0xd3).contains(&input[5]) {
            input[5] - 0xbf
        } else {
            NONE_CGK
        };
        let jong = if input[7] == EUCKR_JAMO_FILLER {
            0
        } else if (0xa1..=0xbe).contains(&input[7]) {
            CGK2U_JONGSEONG[(input[7] - 0xa1) as usize]
        } else {
            NONE_CGK
        };
        if cho == NONE_CGK || jung == NONE_CGK || jong == NONE_CGK {
            return DecodeOne::Illegal(1);
        }
        return DecodeOne::Char(
            0xac00 + u32::from(cho) * 588 + u32::from(jung) * 28 + u32::from(jong),
            8,
        );
    }
    try_map_decode(
        &KSX1001_DECMAP,
        &KSX1001_DECMAP_DATA,
        c ^ 0x80,
        input[1] ^ 0x80,
    )
    .map_or(DecodeOne::Illegal(1), |value| DecodeOne::Char(value, 2))
}

pub(super) fn encode_cp949(c: u32) -> EncodeOne {
    if c < 0x80 {
        let mut output = [0; 8];
        output[0] = c as u8;
        return EncodeOne::Bytes(output, 1, 1);
    }
    let Some(code) = try_map_encode(c) else {
        return EncodeOne::Illegal(1);
    };
    bytes2(
        (code >> 8) as u8 | 0x80,
        if code & 0x8000 != 0 {
            code as u8
        } else {
            code as u8 | 0x80
        },
    )
}

pub(super) fn decode_cp949(input: &[u8]) -> DecodeOne {
    let c = input[0];
    if c < 0x80 {
        return DecodeOne::Char(u32::from(c), 1);
    }
    if input.len() < 2 {
        return DecodeOne::Incomplete;
    }
    let value = try_map_decode(
        &KSX1001_DECMAP,
        &KSX1001_DECMAP_DATA,
        c ^ 0x80,
        input[1] ^ 0x80,
    )
    .or_else(|| try_map_decode(&CP949EXT_DECMAP, &CP949EXT_DECMAP_DATA, c, input[1]));
    value.map_or(DecodeOne::Illegal(1), |value| DecodeOne::Char(value, 2))
}

pub(super) fn encode_johab(mut c: u32) -> EncodeOne {
    if c < 0x80 {
        let mut output = [0; 8];
        output[0] = c as u8;
        return EncodeOne::Bytes(output, 1, 1);
    }
    let code = if (0xac00..=0xd7a3).contains(&c) {
        c -= 0xac00;
        0x8000
            | u16::from(U2JOHABIDX_CHOSEONG[(c / 588) as usize]) << 10
            | u16::from(U2JOHABIDX_JUNGSEONG[((c / 28) % 21) as usize]) << 5
            | u16::from(U2JOHABIDX_JONGSEONG[(c % 28) as usize])
    } else if (0x3131..=0x3163).contains(&c) {
        U2JOHABJAMO[(c - 0x3131) as usize]
    } else {
        let Some(code) = try_map_encode(c) else {
            return EncodeOne::Illegal(1);
        };
        debug_assert_eq!(code & 0x8000, 0);
        let c1 = (code >> 8) as u8;
        let c2 = code as u8;
        if !(((0x21..=0x2c).contains(&c1) || (0x4a..=0x7d).contains(&c1))
            && (0x21..=0x7e).contains(&c2))
        {
            return EncodeOne::Illegal(1);
        }
        let t1 = if c1 < 0x4a {
            u16::from(c1 - 0x21) + 0x1b2
        } else {
            u16::from(c1 - 0x21) + 0x197
        };
        let t2 = (if t1 & 1 != 0 { 0x5e } else { 0 }) + (c2 - 0x21);
        return bytes2(
            (t1 >> 1) as u8,
            if t2 < 0x4e { t2 + 0x31 } else { t2 + 0x43 },
        );
    };
    bytes2((code >> 8) as u8, code as u8)
}

pub(super) fn decode_johab(input: &[u8]) -> DecodeOne {
    let c = input[0];
    if c < 0x80 {
        return DecodeOne::Char(u32::from(c), 1);
    }
    if input.len() < 2 {
        return DecodeOne::Incomplete;
    }
    let c2 = input[1];
    if c < 0xd8 {
        let c_cho = (c >> 2) & 0x1f;
        let c_jung = ((c << 3) | c2 >> 5) & 0x1f;
        let c_jong = c2 & 0x1f;
        let i_cho = JOHABIDX_CHOSEONG[c_cho as usize];
        let i_jung = JOHABIDX_JUNGSEONG[c_jung as usize];
        let i_jong = JOHABIDX_JONGSEONG[c_jong as usize];
        if i_cho == NONE || i_jung == NONE || i_jong == NONE {
            return DecodeOne::Illegal(1);
        }
        let value = if i_cho == FILL {
            if i_jung == FILL {
                if i_jong == FILL {
                    0x3000
                } else {
                    0x3100 | u32::from(JOHABJAMO_JONGSEONG[c_jong as usize])
                }
            } else if i_jong == FILL {
                0x3100 | u32::from(JOHABJAMO_JUNGSEONG[c_jung as usize])
            } else {
                return DecodeOne::Illegal(1);
            }
        } else if i_jung == FILL {
            if i_jong == FILL {
                0x3100 | u32::from(JOHABJAMO_CHOSEONG[c_cho as usize])
            } else {
                return DecodeOne::Illegal(1);
            }
        } else {
            0xac00
                + u32::from(i_cho) * 588
                + u32::from(i_jung) * 28
                + if i_jong == FILL { 0 } else { u32::from(i_jong) }
        };
        return DecodeOne::Char(value, 2);
    }

    if c == 0xdf
        || c > 0xf9
        || c2 < 0x31
        || (0x80..0x91).contains(&c2)
        || c2 & 0x7f == 0x7f
        || (c == 0xda && (0xa1..=0xd3).contains(&c2))
    {
        return DecodeOne::Illegal(1);
    }
    // `_codecs_kr.c::DECODER(johab)` performs this in promoted signed `int`
    // and assigns it to an `unsigned char`; c == 0xd8 therefore wraps -2 to
    // 254 before the page lookup.
    let mut t1 = if c < 0xe0 {
        (2 * (i16::from(c) - 0xd9)) as u8
    } else {
        (2 * u16::from(c) - 0x197) as u8
    };
    let mut t2 = if c2 < 0x91 { c2 - 0x31 } else { c2 - 0x43 };
    t1 = t1
        .wrapping_add(if t2 < 0x5e { 0 } else { 1 })
        .wrapping_add(0x21);
    t2 = (if t2 < 0x5e { t2 } else { t2 - 0x5e }) + 0x21;
    try_map_decode(&KSX1001_DECMAP, &KSX1001_DECMAP_DATA, t1, t2)
        .map_or(DecodeOne::Illegal(1), |value| DecodeOne::Char(value, 2))
}

#[cfg(test)]
mod tests {
    use super::super::{Codec, decode_one, encode_one};
    use super::*;

    fn encode(codec: Codec, text: &str) -> Vec<u8> {
        let mut output = Vec::new();
        for c in text.chars() {
            let points = [c as u32];
            let EncodeOne::Bytes(bytes, len, 1) = encode_one(codec, &points, true, &mut [0; 8])
            else {
                panic!("{c:?} is not encodable");
            };
            output.extend_from_slice(&bytes[..len]);
        }
        output
    }

    fn decode(codec: Codec, input: &[u8]) -> String {
        let mut output = String::new();
        let mut position = 0;
        while position < input.len() {
            let DecodeOne::Char(value, len) = decode_one(codec, &input[position..], &mut [0; 8])
            else {
                panic!("{:x?} is not decodable", &input[position..]);
            };
            output.push(char::from_u32(value).unwrap());
            position += len;
        }
        output
    }

    #[test]
    fn pypy_kr_oracle_vectors() {
        let text = "가힣똠ㄱ漢字";
        let cases = [
            (
                Codec::EucKr,
                &[
                    0xb0, 0xa1, 0xa4, 0xd4, 0xa4, 0xbe, 0xa4, 0xd3, 0xa4, 0xbe, 0xa4, 0xd4, 0xa4,
                    0xa8, 0xa4, 0xc7, 0xa4, 0xb1, 0xa4, 0xa1, 0xf9, 0xd3, 0xed, 0xae,
                ][..],
            ),
            (
                Codec::Cp949,
                &[
                    0xb0, 0xa1, 0xc6, 0x52, 0x8c, 0x63, 0xa4, 0xa1, 0xf9, 0xd3, 0xed, 0xae,
                ][..],
            ),
            (
                Codec::Johab,
                &[
                    0x88, 0x61, 0xd3, 0xbd, 0x99, 0xb1, 0x88, 0x41, 0xf7, 0xd3, 0xf1, 0xae,
                ][..],
            ),
        ];
        for (codec, expected) in cases {
            assert_eq!(encode(codec, text), expected);
            assert_eq!(decode(codec, expected), text);
        }
    }

    #[test]
    fn malformed_candidates_reject_the_lead_byte() {
        for (codec, input) in [
            (Codec::EucKr, &[0xff, 0xff][..]),
            (Codec::Cp949, &[0x81, 0x00][..]),
            (Codec::Johab, &[0x84, 0x00][..]),
            (Codec::Johab, &[0xd8, 0x31][..]),
        ] {
            assert!(matches!(
                decode_one(codec, input, &mut [0; 8]),
                DecodeOne::Illegal(1)
            ));
        }
    }
}
