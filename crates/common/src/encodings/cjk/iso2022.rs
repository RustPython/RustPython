//! Line-by-line Rust port of PyPy cjkcodecs `_codecs_iso2022.c`.

use super::jp::{
    decode_jisx0213_plane1, decode_jisx0213_plane2, emulate_2000_encode_bmp, encode_jisx0213_code,
    find_pair_encode, jisx0201_decode, try_map_decode, try_map_encode,
};
use super::mappings_cn::{
    GB2312_DECMAP, GB2312_DECMAP_DATA, GBCOMMON_ENCMAP, GBCOMMON_ENCMAP_DATA,
};
use super::mappings_jp::{
    JISX0208_DECMAP, JISX0208_DECMAP_DATA, JISX0212_DECMAP, JISX0212_DECMAP_DATA,
    JISX0213_BMP_ENCMAP, JISX0213_BMP_ENCMAP_DATA, JISXCOMMON_ENCMAP, JISXCOMMON_ENCMAP_DATA,
    MULTIC,
};
use super::mappings_kr::{CP949_ENCMAP, CP949_ENCMAP_DATA, KSX1001_DECMAP, KSX1001_DECMAP_DATA};
use super::{Codec, DecodeOne, EncodeOne};

const ESC: u8 = 0x1b;
const SO: u8 = 0x0e;
const SI: u8 = 0x0f;
const LF: u8 = 0x0a;
const MAX_ESCSEQLEN: usize = 16;

const CHARSET_ISO8859_1: u8 = b'A';
const CHARSET_ASCII: u8 = b'B';
const CHARSET_ISO8859_7: u8 = b'F';
const CHARSET_JISX0201_K: u8 = b'I';
const CHARSET_JISX0201_R: u8 = b'J';
const CHARSET_DBCS: u8 = 0x80;
const CHARSET_GB2312: u8 = b'A' | CHARSET_DBCS;
const CHARSET_JISX0208: u8 = b'B' | CHARSET_DBCS;
const CHARSET_KSX1001: u8 = b'C' | CHARSET_DBCS;
const CHARSET_JISX0212: u8 = b'D' | CHARSET_DBCS;
const CHARSET_JISX0213_2000_1: u8 = b'O' | CHARSET_DBCS;
const CHARSET_JISX0213_2: u8 = b'P' | CHARSET_DBCS;
const CHARSET_JISX0213_2004_1: u8 = b'Q' | CHARSET_DBCS;
const CHARSET_JISX0208_O: u8 = b'@' | CHARSET_DBCS;

const F_SHIFTED: u8 = 0x01;
const F_ESCTHROUGHOUT: u8 = 0x02;

const NO_SHIFT: u8 = 0x01;
const USE_G2: u8 = 0x02;
const USE_JISX0208_EXT: u8 = 0x04;

#[derive(Clone, Copy)]
struct Designation {
    mark: u8,
    plane: u8,
    width: u8,
    mapping: Mapping,
}

#[derive(Clone, Copy)]
enum Mapping {
    Ksx1001,
    JisX0201R,
    JisX0201K,
    JisX0208,
    JisX0212,
    JisX0213_2000_1,
    JisX0213_2000_1PairOnly,
    JisX0213_2000_2,
    JisX0213_2004_1,
    JisX0213_2004_1PairOnly,
    JisX0213_2004_2,
    Gb2312,
    Iso8859_1,
    Iso8859_7,
}

const KSX1001_G0: Designation = Designation {
    mark: CHARSET_KSX1001,
    plane: 0,
    width: 2,
    mapping: Mapping::Ksx1001,
};
const KSX1001_G1: Designation = Designation {
    plane: 1,
    ..KSX1001_G0
};
const JISX0201_R: Designation = Designation {
    mark: CHARSET_JISX0201_R,
    plane: 0,
    width: 1,
    mapping: Mapping::JisX0201R,
};
const JISX0201_K: Designation = Designation {
    mark: CHARSET_JISX0201_K,
    plane: 0,
    width: 1,
    mapping: Mapping::JisX0201K,
};
const JISX0208: Designation = Designation {
    mark: CHARSET_JISX0208,
    plane: 0,
    width: 2,
    mapping: Mapping::JisX0208,
};
const JISX0208_O: Designation = Designation {
    mark: CHARSET_JISX0208_O,
    ..JISX0208
};
const JISX0212: Designation = Designation {
    mark: CHARSET_JISX0212,
    plane: 0,
    width: 2,
    mapping: Mapping::JisX0212,
};
const JISX0213_2000_1: Designation = Designation {
    mark: CHARSET_JISX0213_2000_1,
    plane: 0,
    width: 2,
    mapping: Mapping::JisX0213_2000_1,
};
const JISX0213_2000_1_PAIRONLY: Designation = Designation {
    mapping: Mapping::JisX0213_2000_1PairOnly,
    ..JISX0213_2000_1
};
const JISX0213_2000_2: Designation = Designation {
    mark: CHARSET_JISX0213_2,
    plane: 0,
    width: 2,
    mapping: Mapping::JisX0213_2000_2,
};
const JISX0213_2004_1: Designation = Designation {
    mark: CHARSET_JISX0213_2004_1,
    plane: 0,
    width: 2,
    mapping: Mapping::JisX0213_2004_1,
};
const JISX0213_2004_1_PAIRONLY: Designation = Designation {
    mapping: Mapping::JisX0213_2004_1PairOnly,
    ..JISX0213_2004_1
};
const JISX0213_2004_2: Designation = Designation {
    mark: CHARSET_JISX0213_2,
    plane: 0,
    width: 2,
    mapping: Mapping::JisX0213_2004_2,
};
const GB2312: Designation = Designation {
    mark: CHARSET_GB2312,
    plane: 0,
    width: 2,
    mapping: Mapping::Gb2312,
};
const ISO8859_1: Designation = Designation {
    mark: CHARSET_ISO8859_1,
    plane: 2,
    width: 1,
    mapping: Mapping::Iso8859_1,
};
const ISO8859_7: Designation = Designation {
    mark: CHARSET_ISO8859_7,
    plane: 2,
    width: 1,
    mapping: Mapping::Iso8859_7,
};

const KR: &[Designation] = &[KSX1001_G1];
const JP: &[Designation] = &[JISX0208, JISX0201_R, JISX0208_O];
const JP_1: &[Designation] = &[JISX0208, JISX0212, JISX0201_R, JISX0208_O];
const JP_2: &[Designation] = &[
    JISX0208, JISX0212, KSX1001_G0, GB2312, JISX0201_R, JISX0208_O, ISO8859_1, ISO8859_7,
];
const JP_2004: &[Designation] = &[
    JISX0213_2004_1_PAIRONLY,
    JISX0208,
    JISX0213_2004_1,
    JISX0213_2004_2,
];
const JP_3: &[Designation] = &[
    JISX0213_2000_1_PAIRONLY,
    JISX0208,
    JISX0213_2000_1,
    JISX0213_2000_2,
];
const JP_EXT: &[Designation] = &[JISX0208, JISX0212, JISX0201_R, JISX0201_K, JISX0208_O];

fn config(codec: Codec) -> (u8, &'static [Designation]) {
    match codec {
        Codec::Iso2022Kr => (0, KR),
        Codec::Iso2022Jp => (NO_SHIFT | USE_JISX0208_EXT, JP),
        Codec::Iso2022Jp1 => (NO_SHIFT | USE_JISX0208_EXT, JP_1),
        Codec::Iso2022Jp2 => (NO_SHIFT | USE_G2 | USE_JISX0208_EXT, JP_2),
        Codec::Iso2022Jp2004 => (NO_SHIFT | USE_JISX0208_EXT, JP_2004),
        Codec::Iso2022Jp3 => (NO_SHIFT | USE_JISX0208_EXT, JP_3),
        Codec::Iso2022JpExt => (NO_SHIFT | USE_JISX0208_EXT, JP_EXT),
        _ => unreachable!("ISO-2022 dispatcher received another codec"),
    }
}

pub(super) fn prepare_decode_state(state: &mut [u8; 8]) {
    if state.iter().all(|&byte| byte == 0) {
        state[0] = CHARSET_ASCII;
        state[1] = CHARSET_ASCII;
        state[2] = CHARSET_ASCII;
    }
}

pub(super) fn prepare_encode_state(state: &mut [u8; 8]) {
    if state.iter().all(|&byte| byte == 0) {
        state[0] = CHARSET_ASCII;
        state[1] = CHARSET_ASCII;
    }
}

fn raw_map(index: &[super::mappings_kr::MapIndex; 256], data: &[u16], c: u32) -> Option<u16> {
    if c > 0xffff {
        return None;
    }
    let page = index[(c >> 8) as usize];
    let low = c as u8;
    if !page.present || low < page.bottom || low > page.top {
        return None;
    }
    let value = data[page.offset + usize::from(low - page.bottom)];
    (value != super::mappings_kr::NOCHAR).then_some(value)
}

fn raw_map_cn(index: &[super::mappings_cn::MapIndex; 256], data: &[u16], c: u32) -> Option<u16> {
    if c > 0xffff {
        return None;
    }
    let page = index[(c >> 8) as usize];
    let low = c as u8;
    if !page.present || low < page.bottom || low > page.top {
        return None;
    }
    let value = data[page.offset + usize::from(low - page.bottom)];
    (value != super::mappings_cn::NOCHAR).then_some(value)
}

fn raw_decode_kr(
    index: &[super::mappings_kr::MapIndex; 256],
    data: &[u16],
    c1: u8,
    c2: u8,
) -> Option<u32> {
    let page = index[c1 as usize];
    if !page.present || c2 < page.bottom || c2 > page.top {
        return None;
    }
    let value = data[page.offset + usize::from(c2 - page.bottom)];
    (value != super::mappings_kr::UNIINV).then_some(u32::from(value))
}

fn raw_decode_cn(
    index: &[super::mappings_cn::MapIndex; 256],
    data: &[u16],
    c1: u8,
    c2: u8,
) -> Option<u32> {
    let page = index[c1 as usize];
    if !page.present || c2 < page.bottom || c2 > page.top {
        return None;
    }
    let value = data[page.offset + usize::from(c2 - page.bottom)];
    (value != super::mappings_cn::UNIINV).then_some(u32::from(value))
}

fn encode_pair_only(
    input: &[u32],
    final_input: bool,
    config_2000: bool,
) -> Result<(u16, usize), EncodeOne> {
    let c = input[0];
    if c > 0xffff {
        return Err(EncodeOne::Illegal(1));
    }
    if config_2000 && emulate_2000_encode_bmp(c).is_err() {
        return Err(EncodeOne::Illegal(1));
    }
    if try_map_encode(&JISX0213_BMP_ENCMAP, &JISX0213_BMP_ENCMAP_DATA, c) != Some(MULTIC) {
        return Err(EncodeOne::Illegal(1));
    }
    if input.len() < 2 {
        return if final_input {
            Err(EncodeOne::Illegal(1))
        } else {
            Err(EncodeOne::Incomplete)
        };
    }
    if input[1] == 0 {
        return Err(EncodeOne::Illegal(1));
    }
    find_pair_encode(c, input[1])
        .map(|code| (code, 2))
        .ok_or(EncodeOne::Illegal(1))
}

fn encode_mapping(
    mapping: Mapping,
    input: &[u32],
    final_input: bool,
) -> Result<(u16, usize), EncodeOne> {
    let c = input[0];
    let mapped = match mapping {
        Mapping::Ksx1001 => {
            raw_map(&CP949_ENCMAP, &CP949_ENCMAP_DATA, c).filter(|code| code & 0x8000 == 0)
        }
        Mapping::JisX0201R => {
            if c < 0x80 && c != 0x5c && c != 0x7e {
                Some(c as u16)
            } else if c == 0x00a5 {
                Some(0x5c)
            } else if c == 0x203e {
                Some(0x7e)
            } else {
                None
            }
        }
        Mapping::JisX0201K => (0xff61..=0xff9f).contains(&c).then(|| (c - 0xff40) as u16),
        Mapping::JisX0208 => {
            if c == 0xff3c {
                Some(0x2140)
            } else {
                try_map_encode(&JISXCOMMON_ENCMAP, &JISXCOMMON_ENCMAP_DATA, c)
                    .filter(|code| code & 0x8000 == 0)
            }
        }
        Mapping::JisX0212 => try_map_encode(&JISXCOMMON_ENCMAP, &JISXCOMMON_ENCMAP_DATA, c)
            .filter(|code| code & 0x8000 != 0)
            .map(|code| code & 0x7fff),
        Mapping::JisX0213_2000_1PairOnly => return encode_pair_only(input, final_input, true),
        Mapping::JisX0213_2004_1PairOnly => return encode_pair_only(input, final_input, false),
        Mapping::JisX0213_2000_1
        | Mapping::JisX0213_2000_2
        | Mapping::JisX0213_2004_1
        | Mapping::JisX0213_2004_2 => {
            let config_2000 =
                matches!(mapping, Mapping::JisX0213_2000_1 | Mapping::JisX0213_2000_2);
            let (code, consumed) = encode_jisx0213_code(input, final_input, config_2000, false)?;
            let plane2 = matches!(mapping, Mapping::JisX0213_2000_2 | Mapping::JisX0213_2004_2);
            if plane2 == (code & 0x8000 != 0) {
                return Ok((code & 0x7fff, consumed));
            }
            None
        }
        Mapping::Gb2312 => {
            raw_map_cn(&GBCOMMON_ENCMAP, &GBCOMMON_ENCMAP_DATA, c).filter(|code| code & 0x8000 == 0)
        }
        Mapping::Iso8859_1 | Mapping::Iso8859_7 => None,
    };
    mapped.map(|code| (code, 1)).ok_or(EncodeOne::Illegal(1))
}

fn append(output: &mut [u8; 8], length: &mut usize, bytes: &[u8]) {
    output[*length..*length + bytes.len()].copy_from_slice(bytes);
    *length += bytes.len();
}

pub(super) fn encode_one(
    codec: Codec,
    input: &[u32],
    final_input: bool,
    state: &mut [u8; 8],
) -> EncodeOne {
    prepare_encode_state(state);
    let (_, designations) = config(codec);
    let c = input[0];
    let mut output = [0; 8];
    let mut length = 0;

    if c < 0x80 {
        if state[0] != CHARSET_ASCII {
            append(&mut output, &mut length, &[ESC, b'(', b'B']);
            state[0] = CHARSET_ASCII;
        }
        if state[4] & F_SHIFTED != 0 {
            append(&mut output, &mut length, &[SI]);
            state[4] &= !F_SHIFTED;
        }
        append(&mut output, &mut length, &[c as u8]);
        return EncodeOne::Bytes(output, length, 1);
    }

    let mut selected = None;
    for designation in designations {
        match encode_mapping(designation.mapping, input, final_input) {
            Ok((code, consumed)) => {
                selected = Some((*designation, code, consumed));
                break;
            }
            Err(EncodeOne::Incomplete) => return EncodeOne::Incomplete,
            Err(_) => {}
        }
    }
    let Some((designation, encoded, consumed)) = selected else {
        return EncodeOne::Illegal(1);
    };

    match designation.plane {
        0 => {
            if state[4] & F_SHIFTED != 0 {
                append(&mut output, &mut length, &[SI]);
                state[4] &= !F_SHIFTED;
            }
            if state[0] != designation.mark {
                if designation.width == 1 {
                    append(
                        &mut output,
                        &mut length,
                        &[ESC, b'(', designation.mark & 0x7f],
                    );
                } else if designation.mark == CHARSET_JISX0208 {
                    append(
                        &mut output,
                        &mut length,
                        &[ESC, b'$', designation.mark & 0x7f],
                    );
                } else {
                    append(
                        &mut output,
                        &mut length,
                        &[ESC, b'$', b'(', designation.mark & 0x7f],
                    );
                }
                state[0] = designation.mark;
            }
        }
        1 => {
            if state[1] != designation.mark {
                if designation.width == 1 {
                    append(
                        &mut output,
                        &mut length,
                        &[ESC, b')', designation.mark & 0x7f],
                    );
                } else {
                    append(
                        &mut output,
                        &mut length,
                        &[ESC, b'$', b')', designation.mark & 0x7f],
                    );
                }
                state[1] = designation.mark;
            }
            if state[4] & F_SHIFTED == 0 {
                append(&mut output, &mut length, &[SO]);
                state[4] |= F_SHIFTED;
            }
        }
        _ => return EncodeOne::Illegal(1),
    }

    if designation.width == 1 {
        append(&mut output, &mut length, &[encoded as u8]);
    } else {
        append(
            &mut output,
            &mut length,
            &[(encoded >> 8) as u8, encoded as u8],
        );
    }
    EncodeOne::Bytes(output, length, consumed)
}

pub(super) fn encode_reset(state: &mut [u8; 8]) -> Option<([u8; 8], usize)> {
    prepare_encode_state(state);
    let mut output = [0; 8];
    let mut length = 0;
    if state[4] & F_SHIFTED != 0 {
        append(&mut output, &mut length, &[SI]);
        state[4] &= !F_SHIFTED;
    }
    if state[0] != CHARSET_ASCII {
        append(&mut output, &mut length, &[ESC, b'(', b'B']);
        state[0] = CHARSET_ASCII;
    }
    (length != 0).then_some((output, length))
}

fn is_esc_end(c: u8) -> bool {
    c.is_ascii_uppercase() || c == b'@'
}

fn is_iso2022_esc(c: u8) -> bool {
    matches!(c, b'(' | b')' | b'$' | b'.' | b'&')
}

fn process_escape(codec: Codec, input: &[u8], state: &mut [u8; 8]) -> DecodeOne {
    let (flags, designations) = config(codec);
    let mut i = 1;
    let escape_length = loop {
        if i >= MAX_ESCSEQLEN {
            return DecodeOne::Illegal(1);
        }
        if i >= input.len() {
            return DecodeOne::Incomplete;
        }
        if is_esc_end(input[i]) {
            break i + 1;
        }
        if flags & USE_JISX0208_EXT != 0
            && i + 1 < input.len()
            && input[i] == b'&'
            && input[i + 1] == b'@'
        {
            i += 2;
        }
        i += 1;
    };

    let (charset, plane) = match escape_length {
        3 if input[1] == b'$' => (input[2] | CHARSET_DBCS, 0),
        3 => {
            let plane = match input[1] {
                b'(' => 0,
                b')' => 1,
                b'.' if flags & USE_G2 != 0 => 2,
                _ => return DecodeOne::Illegal(3),
            };
            (input[2], plane)
        }
        4 if input[1] == b'$' => {
            let plane = match input[2] {
                b'(' => 0,
                b')' => 1,
                _ => return DecodeOne::Illegal(4),
            };
            (input[3] | CHARSET_DBCS, plane)
        }
        4 => return DecodeOne::Illegal(4),
        6 if flags & USE_JISX0208_EXT != 0
            && input[3] == ESC
            && input[4] == b'$'
            && input[5] == b'B' =>
        {
            (CHARSET_JISX0208, 0)
        }
        length => return DecodeOne::Illegal(length),
    };
    if charset != CHARSET_ASCII && !designations.iter().any(|dsg| dsg.mark == charset) {
        return DecodeOne::Illegal(escape_length);
    }
    state[plane] = charset;
    DecodeOne::Skip(escape_length)
}

fn decode_iso8859_7(c: u8) -> Option<u32> {
    if c < 0xa0 || (c < 0xc0 && 0x288f3bc9_u32 & (1 << (c - 0xa0)) != 0) {
        Some(u32::from(c))
    } else if (0xb4..=0xfe).contains(&c) && (c >= 0xd4 || 0xbffffd77_u64 & (1 << (c - 0xb4)) != 0) {
        Some(0x02d0 + u32::from(c))
    } else {
        match c {
            0xa1 => Some(0x2018),
            0xa2 => Some(0x2019),
            0xaf => Some(0x2015),
            _ => None,
        }
    }
}

fn process_g2(input: &[u8], state: &[u8; 8]) -> DecodeOne {
    if input.len() < 3 {
        return DecodeOne::Incomplete;
    }
    let c = input[2];
    let decoded = match state[2] {
        CHARSET_ISO8859_1 if c < 0x80 => Some(u32::from(c) + 0x80),
        CHARSET_ISO8859_7 => decode_iso8859_7(c ^ 0x80),
        CHARSET_ASCII if c & 0x80 == 0 => Some(u32::from(c)),
        _ => None,
    };
    decoded.map_or(DecodeOne::Illegal(3), |value| DecodeOne::Char(value, 3))
}

fn decode_mapping(mapping: Mapping, input: &[u8], width: usize) -> DecodeOne {
    let c1 = input[0];
    let c2 = input.get(1).copied().unwrap_or(0);
    let mapped = match mapping {
        Mapping::Ksx1001 => raw_decode_kr(&KSX1001_DECMAP, &KSX1001_DECMAP_DATA, c1, c2),
        Mapping::JisX0201R => jisx0201_decode(c1),
        Mapping::JisX0201K => {
            jisx0201_decode(c1 ^ 0x80).filter(|value| (0xff61..=0xff9f).contains(value))
        }
        Mapping::JisX0208 => {
            if (c1, c2) == (0x21, 0x40) {
                Some(0xff3c)
            } else {
                try_map_decode(&JISX0208_DECMAP, &JISX0208_DECMAP_DATA, c1, c2)
            }
        }
        Mapping::JisX0212 => try_map_decode(&JISX0212_DECMAP, &JISX0212_DECMAP_DATA, c1, c2),
        Mapping::JisX0213_2000_1 | Mapping::JisX0213_2000_1PairOnly => {
            return match decode_jisx0213_plane1(c1, c2, true, width, true, false) {
                DecodeOne::Illegal(_) => DecodeOne::Illegal(width),
                decoded => decoded,
            };
        }
        Mapping::JisX0213_2000_2 => {
            // PyPy `_codecs_iso2022.c::jisx0213_2000_2_decoder` expands
            // `EMULATE_JISX0213_2000_DECODE_PLANE2` and then, deliberately
            // without `else`, runs the 2004 table lookup.  Thus row 0x7d,
            // cell 0x3b is first assigned U+9B1D and then overwritten by the
            // table's U+9B1C.  The EUC/Shift-JIS decoders put `else` there and
            // must keep U+9B1D, so preserve the ISO-only ordering here.
            let table = decode_jisx0213_plane2(c1, c2, false, width);
            if !matches!(table, DecodeOne::Illegal(_)) {
                return table;
            }
            return match decode_jisx0213_plane2(c1, c2, true, width) {
                DecodeOne::Illegal(_) => DecodeOne::Illegal(width),
                decoded => decoded,
            };
        }
        Mapping::JisX0213_2004_1 | Mapping::JisX0213_2004_1PairOnly => {
            return match decode_jisx0213_plane1(c1, c2, false, width, true, false) {
                DecodeOne::Illegal(_) => DecodeOne::Illegal(width),
                decoded => decoded,
            };
        }
        Mapping::JisX0213_2004_2 => {
            return match decode_jisx0213_plane2(c1, c2, false, width) {
                DecodeOne::Illegal(_) => DecodeOne::Illegal(width),
                decoded => decoded,
            };
        }
        Mapping::Gb2312 => raw_decode_cn(&GB2312_DECMAP, &GB2312_DECMAP_DATA, c1, c2),
        Mapping::Iso8859_1 | Mapping::Iso8859_7 => None,
    };
    mapped.map_or(DecodeOne::Illegal(width), |value| {
        DecodeOne::Char(value, width)
    })
}

pub(super) fn decode_one(codec: Codec, input: &[u8], state: &mut [u8; 8]) -> DecodeOne {
    prepare_decode_state(state);
    let (flags, designations) = config(codec);
    let c = input[0];

    if state[4] & F_ESCTHROUGHOUT != 0 {
        if is_esc_end(c) {
            state[4] &= !F_ESCTHROUGHOUT;
        }
        return DecodeOne::Char(u32::from(c), 1);
    }

    match c {
        ESC => {
            if input.len() < 2 {
                DecodeOne::Incomplete
            } else if is_iso2022_esc(input[1]) {
                process_escape(codec, input, state)
            } else if flags & USE_G2 != 0 && input[1] == b'N' {
                process_g2(input, state)
            } else {
                state[4] |= F_ESCTHROUGHOUT;
                DecodeOne::Char(u32::from(ESC), 1)
            }
        }
        SI if flags & NO_SHIFT == 0 => {
            state[4] &= !F_SHIFTED;
            DecodeOne::Skip(1)
        }
        SO if flags & NO_SHIFT == 0 => {
            state[4] |= F_SHIFTED;
            DecodeOne::Skip(1)
        }
        LF => {
            state[4] &= !F_SHIFTED;
            DecodeOne::Char(u32::from(LF), 1)
        }
        _ if c < 0x20 => DecodeOne::Char(u32::from(c), 1),
        _ if c >= 0x80 => DecodeOne::Illegal(1),
        _ => {
            let charset = if state[4] & F_SHIFTED != 0 {
                state[1]
            } else {
                state[0]
            };
            if charset == CHARSET_ASCII {
                return DecodeOne::Char(u32::from(c), 1);
            }
            let Some(designation) = designations.iter().find(|dsg| dsg.mark == charset) else {
                return DecodeOne::Illegal(1);
            };
            let width = usize::from(designation.width);
            if input.len() < width {
                return DecodeOne::Incomplete;
            }
            decode_mapping(designation.mapping, input, width)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(codec: Codec, text: &str) -> Vec<u8> {
        let input: Vec<u32> = text.chars().map(u32::from).collect();
        let mut state = [0; 8];
        let mut output = Vec::new();
        let mut position = 0;
        while position < input.len() {
            let EncodeOne::Bytes(bytes, length, consumed) =
                encode_one(codec, &input[position..], true, &mut state)
            else {
                panic!("U+{:04X} is not encodable", input[position]);
            };
            output.extend_from_slice(&bytes[..length]);
            position += consumed;
        }
        if let Some((bytes, length)) = encode_reset(&mut state) {
            output.extend_from_slice(&bytes[..length]);
        }
        output
    }

    fn decode(codec: Codec, input: &[u8]) -> String {
        let mut state = [0; 8];
        let mut output = String::new();
        let mut position = 0;
        while position < input.len() {
            match decode_one(codec, &input[position..], &mut state) {
                DecodeOne::Char(value, consumed) => {
                    output.push(char::from_u32(value).unwrap());
                    position += consumed;
                }
                DecodeOne::Pair(first, second, consumed) => {
                    output.push(char::from_u32(first).unwrap());
                    output.push(char::from_u32(second).unwrap());
                    position += consumed;
                }
                DecodeOne::Skip(consumed) => position += consumed,
                DecodeOne::Incomplete | DecodeOne::Illegal(_) => {
                    panic!("input is not decodable at byte {position}")
                }
            }
        }
        output
    }

    #[test]
    fn pypy_iso2022_oracle_vectors() {
        let vectors = [
            (Codec::Iso2022Kr, "한국어", &b"\x1b$)C\x0eGQ19>n\x0f"[..]),
            (Codec::Iso2022Jp, "日本語", &b"\x1b$BF|K\\8l\x1b(B"[..]),
            (Codec::Iso2022Jp1, "日本語", &b"\x1b$BF|K\\8l\x1b(B"[..]),
            (Codec::Iso2022Jp2, "한국어", &b"\x1b$(CGQ19>n\x1b(B"[..]),
            (
                Codec::Iso2022Jp2004,
                "か\u{309a}\u{2000b}",
                &b"\x1b$(Q$w.\x22\x1b(B"[..],
            ),
            (
                Codec::Iso2022Jp3,
                "か\u{309a}\u{2000b}",
                &b"\x1b$(O$w.\x22\x1b(B"[..],
            ),
            (Codec::Iso2022JpExt, "日本語", &b"\x1b$BF|K\\8l\x1b(B"[..]),
        ];
        for (codec, text, expected) in vectors {
            assert_eq!(encode(codec, text), expected, "{codec:?}");
            assert_eq!(decode(codec, expected), text, "{codec:?}");
        }
        assert_eq!(
            encode(Codec::Iso2022Jp2, "¥‾\\～"),
            b"\x1b(J\\~\x1b(B\\\x1b$(C\x22&\x1b(B"
        );
        assert_eq!(
            decode(Codec::Iso2022Jp2, b"\x1b(B:hu4:unit\x1b.A\x1bNi de famille"),
            ":hu4:unit\u{e9} de famille"
        );
        assert_eq!(decode(Codec::Iso2022Jp, b"\x1bXabcZ"), "\x1bXabcZ");
        assert_eq!(decode(Codec::Iso2022Jp2004, b"\x1b$(Q\x22\x32\x1b(B"), "~");
        assert_eq!(decode(Codec::Iso2022Jp3, b"\x1b$(O\x22\x32\x1b(B"), "~");
        assert_eq!(
            decode(Codec::Iso2022Jp3, b"\x1b$(P\x7d\x3b\x1b(B"),
            "\u{9b1c}"
        );
        assert!(encode(Codec::Iso2022Jp2004, "か\0").ends_with(&[0]));
        assert!(encode(Codec::Iso2022Jp3, "か\0").ends_with(&[0]));
    }
}
