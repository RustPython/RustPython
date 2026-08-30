//! Rust ports of PyPy's `pypy/module/_multibytecodec/src/cjkcodecs` units.
//!
//! This module contains only the VM-independent codec engines and mapping
//! tables. Interpreter-specific objects, buffers, and error handlers belong in
//! the VM adapter that calls this API.

// Keep the control flow and arithmetic visibly aligned with PyPy's C engines.
#![allow(
    clippy::bool_to_int_with_if,
    clippy::collapsible_if,
    clippy::trivially_copy_pass_by_ref,
    clippy::unnested_or_patterns
)]

mod cn;
mod hk;
mod iso2022;
mod jp;
mod kr;
mod mappings_cn;
mod mappings_hk;
mod mappings_jisx0213_pair;
mod mappings_jp;
mod mappings_kr;
mod mappings_tw;
mod tw;

/// A VM-independent CJK codec implemented by this module.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Codec {
    EucKr,
    Cp949,
    Johab,
    Big5,
    Cp950,
    Big5Hkscs,
    ShiftJis,
    Cp932,
    EucJp,
    ShiftJis2004,
    EucJis2004,
    EucJisX0213,
    ShiftJisX0213,
    Iso2022Kr,
    Iso2022Jp,
    Iso2022Jp1,
    Iso2022Jp2,
    Iso2022Jp2004,
    Iso2022Jp3,
    Iso2022JpExt,
    Gb2312,
    Gbk,
    Gb18030,
    Hz,
}

impl Codec {
    /// Look up the canonical codec name used by Python's CJK codec modules.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "euc_kr" => Some(Self::EucKr),
            "cp949" => Some(Self::Cp949),
            "johab" => Some(Self::Johab),
            "big5" => Some(Self::Big5),
            "cp950" => Some(Self::Cp950),
            "big5hkscs" => Some(Self::Big5Hkscs),
            "shift_jis" => Some(Self::ShiftJis),
            "cp932" => Some(Self::Cp932),
            "euc_jp" => Some(Self::EucJp),
            "shift_jis_2004" => Some(Self::ShiftJis2004),
            "euc_jis_2004" => Some(Self::EucJis2004),
            "euc_jisx0213" => Some(Self::EucJisX0213),
            "shift_jisx0213" => Some(Self::ShiftJisX0213),
            "iso2022_kr" => Some(Self::Iso2022Kr),
            "iso2022_jp" => Some(Self::Iso2022Jp),
            "iso2022_jp_1" => Some(Self::Iso2022Jp1),
            "iso2022_jp_2" => Some(Self::Iso2022Jp2),
            "iso2022_jp_2004" => Some(Self::Iso2022Jp2004),
            "iso2022_jp_3" => Some(Self::Iso2022Jp3),
            "iso2022_jp_ext" => Some(Self::Iso2022JpExt),
            "gb2312" => Some(Self::Gb2312),
            "gbk" => Some(Self::Gbk),
            "gb18030" => Some(Self::Gb18030),
            "hz" => Some(Self::Hz),
            _ => None,
        }
    }
}

fn is_iso2022(codec: Codec) -> bool {
    matches!(
        codec,
        Codec::Iso2022Kr
            | Codec::Iso2022Jp
            | Codec::Iso2022Jp1
            | Codec::Iso2022Jp2
            | Codec::Iso2022Jp2004
            | Codec::Iso2022Jp3
            | Codec::Iso2022JpExt
    )
}

/// Construct the initial opaque state for an encoder or decoder.
#[must_use]
pub fn initial_state(codec: Codec, decoder: bool) -> [u8; 8] {
    let mut state = [0; 8];
    if is_iso2022(codec) {
        if decoder {
            iso2022::prepare_decode_state(&mut state);
        } else {
            iso2022::prepare_encode_state(&mut state);
        }
    }
    state
}

/// The result of decoding one engine step.
pub enum DecodeOne {
    Char(u32, usize),
    Pair(u32, u32, usize),
    Skip(usize),
    Incomplete,
    Illegal(usize),
}

/// The result of encoding one engine step.
pub enum EncodeOne {
    Bytes([u8; 8], usize, usize),
    Incomplete,
    Illegal(usize),
}

/// Decode one engine step from `input`, updating the opaque codec `state`.
pub fn decode_one(codec: Codec, input: &[u8], state: &mut [u8; 8]) -> DecodeOne {
    if input.is_empty() {
        return DecodeOne::Incomplete;
    }
    if is_iso2022(codec) {
        return iso2022::decode_one(codec, input, state);
    }
    match codec {
        Codec::EucKr => kr::decode_euc_kr(input),
        Codec::Cp949 => kr::decode_cp949(input),
        Codec::Johab => kr::decode_johab(input),
        Codec::Big5 => tw::decode_big5(input),
        Codec::Cp950 => tw::decode_cp950(input),
        Codec::Big5Hkscs => hk::decode_big5hkscs(input),
        Codec::ShiftJis => jp::decode_shift_jis(input),
        Codec::Cp932 => jp::decode_cp932(input),
        Codec::EucJp => jp::decode_euc_jp(input),
        Codec::ShiftJis2004 => jp::decode_shift_jis_2004(input, false),
        Codec::EucJis2004 => jp::decode_euc_jis_2004(input, false),
        Codec::EucJisX0213 => jp::decode_euc_jis_2004(input, true),
        Codec::ShiftJisX0213 => jp::decode_shift_jis_2004(input, true),
        Codec::Gb2312 => cn::decode_gb2312(input),
        Codec::Gbk => cn::decode_gbk(input),
        Codec::Gb18030 => cn::decode_gb18030(input),
        Codec::Hz => cn::decode_hz(input, state),
        Codec::Iso2022Kr
        | Codec::Iso2022Jp
        | Codec::Iso2022Jp1
        | Codec::Iso2022Jp2
        | Codec::Iso2022Jp2004
        | Codec::Iso2022Jp3
        | Codec::Iso2022JpExt => unreachable!(),
    }
}

/// Encode one engine step from Unicode scalar values.
pub fn encode_one(
    codec: Codec,
    input: &[u32],
    final_input: bool,
    state: &mut [u8; 8],
) -> EncodeOne {
    if input.is_empty() {
        return EncodeOne::Incomplete;
    }
    if is_iso2022(codec) {
        return iso2022::encode_one(codec, input, final_input, state);
    }
    match codec {
        Codec::EucKr => kr::encode_euc_kr(input[0]),
        Codec::Cp949 => kr::encode_cp949(input[0]),
        Codec::Johab => kr::encode_johab(input[0]),
        Codec::Big5 => tw::encode_big5(input[0]),
        Codec::Cp950 => tw::encode_cp950(input[0]),
        Codec::Big5Hkscs => hk::encode_big5hkscs(input, final_input),
        Codec::ShiftJis => jp::encode_shift_jis(input[0]),
        Codec::Cp932 => jp::encode_cp932(input[0]),
        Codec::EucJp => jp::encode_euc_jp(input[0]),
        Codec::ShiftJis2004 => jp::encode_shift_jis_2004(input, final_input, false),
        Codec::EucJis2004 => jp::encode_euc_jis_2004(input, final_input, false),
        Codec::EucJisX0213 => jp::encode_euc_jis_2004(input, final_input, true),
        Codec::ShiftJisX0213 => jp::encode_shift_jis_2004(input, final_input, true),
        Codec::Gb2312 => cn::encode_gb2312(input[0]),
        Codec::Gbk => cn::encode_gbk(input[0]),
        Codec::Gb18030 => cn::encode_gb18030(input[0]),
        Codec::Hz => cn::encode_hz(input[0], state),
        Codec::Iso2022Kr
        | Codec::Iso2022Jp
        | Codec::Iso2022Jp1
        | Codec::Iso2022Jp2
        | Codec::Iso2022Jp2004
        | Codec::Iso2022Jp3
        | Codec::Iso2022JpExt => unreachable!(),
    }
}

/// Flush an encoder's shift state, if the selected codec has one.
pub fn encode_reset(codec: Codec, state: &mut [u8; 8]) -> Option<([u8; 8], usize)> {
    match codec {
        Codec::Hz => cn::reset_hz(state),
        Codec::Iso2022Kr
        | Codec::Iso2022Jp
        | Codec::Iso2022Jp1
        | Codec::Iso2022Jp2
        | Codec::Iso2022Jp2004
        | Codec::Iso2022Jp3
        | Codec::Iso2022JpExt => iso2022::encode_reset(state),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_is_incomplete() {
        assert!(matches!(
            decode_one(Codec::EucKr, &[], &mut [0; 8]),
            DecodeOne::Incomplete
        ));
        assert!(matches!(
            encode_one(Codec::EucKr, &[], true, &mut [0; 8]),
            EncodeOne::Incomplete
        ));
    }

    #[test]
    fn every_one_and_two_byte_candidate_is_total() {
        let codecs = [
            Codec::EucKr,
            Codec::Cp949,
            Codec::Johab,
            Codec::Big5,
            Codec::Cp950,
            Codec::Big5Hkscs,
            Codec::ShiftJis,
            Codec::Cp932,
            Codec::EucJp,
            Codec::ShiftJis2004,
            Codec::EucJis2004,
            Codec::EucJisX0213,
            Codec::ShiftJisX0213,
            Codec::Iso2022Kr,
            Codec::Iso2022Jp,
            Codec::Iso2022Jp1,
            Codec::Iso2022Jp2,
            Codec::Iso2022Jp2004,
            Codec::Iso2022Jp3,
            Codec::Iso2022JpExt,
            Codec::Gb2312,
            Codec::Gbk,
            Codec::Gb18030,
            Codec::Hz,
        ];
        for codec in codecs {
            for first in 0..=u8::MAX {
                let _ = decode_one(codec, &[first], &mut [0; 8]);
                for second in 0..=u8::MAX {
                    let _ = decode_one(codec, &[first, second], &mut [0; 8]);
                }
            }
        }
    }
}
