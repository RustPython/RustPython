use std::ops::Range;

use num_traits::ToPrimitive;

use crate::str::StrKind;
use crate::wtf8::{Wtf8, Wtf8Buf};

pub type EncodeErrorResult<S, B, E> = Result<(EncodeReplace<S, B>, usize), E>;

pub type DecodeErrorResult<S, B, E> = Result<(S, Option<B>, usize), E>;

pub trait StrBuffer: AsRef<Wtf8> {
    fn is_compatible_with(&self, kind: StrKind) -> bool {
        let s = self.as_ref();
        match kind {
            StrKind::Ascii => s.is_ascii(),
            StrKind::Utf8 => s.is_utf8(),
            StrKind::Wtf8 => true,
        }
    }
}

pub trait ErrorHandler {
    type Error;
    type StrBuf: StrBuffer;
    type BytesBuf: AsRef<[u8]>;
    fn handle_encode_error(
        &self,
        data: &Wtf8,
        char_range: Range<usize>,
        reason: &str,
    ) -> EncodeErrorResult<Self::StrBuf, Self::BytesBuf, Self::Error>;
    fn handle_decode_error(
        &self,
        data: &[u8],
        byte_range: Range<usize>,
        reason: &str,
    ) -> DecodeErrorResult<Self::StrBuf, Self::BytesBuf, Self::Error>;
    fn error_oob_restart(&self, i: usize) -> Self::Error;
    fn error_encoding(&self, data: &Wtf8, char_range: Range<usize>, reason: &str) -> Self::Error;
}
pub enum EncodeReplace<S, B> {
    Str(S),
    Bytes(B),
}

struct DecodeError<'a> {
    valid_prefix: &'a str,
    rest: &'a [u8],
    err_len: Option<usize>,
}
/// # Safety
/// `v[..valid_up_to]` must be valid utf8
unsafe fn make_decode_err(v: &[u8], valid_up_to: usize, err_len: Option<usize>) -> DecodeError<'_> {
    let (valid_prefix, rest) = unsafe { v.split_at_unchecked(valid_up_to) };
    let valid_prefix = unsafe { core::str::from_utf8_unchecked(valid_prefix) };
    DecodeError {
        valid_prefix,
        rest,
        err_len,
    }
}

enum HandleResult<'a> {
    Done,
    Error {
        err_len: Option<usize>,
        reason: &'a str,
    },
}
fn decode_utf8_compatible<E: ErrorHandler, DecodeF, ErrF>(
    data: &[u8],
    errors: &E,
    decode: DecodeF,
    handle_error: ErrF,
) -> Result<(Wtf8Buf, usize), E::Error>
where
    DecodeF: Fn(&[u8]) -> Result<&str, DecodeError<'_>>,
    ErrF: Fn(&[u8], Option<usize>) -> HandleResult<'_>,
{
    if data.is_empty() {
        return Ok((Wtf8Buf::new(), 0));
    }
    // we need to coerce the lifetime to that of the function body rather than the
    // anonymous input lifetime, so that we can assign it data borrowed from data_from_err
    let mut data = data;
    let mut data_from_err: E::BytesBuf;
    let mut out = Wtf8Buf::with_capacity(data.len());
    let mut remaining_index = 0;
    let mut remaining_data = data;
    loop {
        match decode(remaining_data) {
            Ok(decoded) => {
                out.push_str(decoded);
                remaining_index += decoded.len();
                break;
            }
            Err(e) => {
                out.push_str(e.valid_prefix);
                match handle_error(e.rest, e.err_len) {
                    HandleResult::Done => {
                        remaining_index += e.valid_prefix.len();
                        break;
                    }
                    HandleResult::Error { err_len, reason } => {
                        let err_idx = remaining_index + e.valid_prefix.len();
                        let err_range =
                            err_idx..err_len.map_or_else(|| data.len(), |len| err_idx + len);
                        let (replace, new_data, restart) =
                            errors.handle_decode_error(data, err_range, reason)?;
                        out.push_wtf8(replace.as_ref());
                        if let Some(new_data) = new_data {
                            data_from_err = new_data;
                            data = data_from_err.as_ref();
                        }
                        remaining_data = data
                            .get(restart..)
                            .ok_or_else(|| errors.error_oob_restart(restart))?;
                        remaining_index = restart;
                        continue;
                    }
                }
            }
        }
    }
    Ok((out, remaining_index))
}

#[inline]
fn encode_utf8_compatible<E: ErrorHandler>(
    s: &Wtf8,
    errors: &E,
    err_reason: &str,
    target_kind: StrKind,
) -> Result<Vec<u8>, E::Error> {
    let full_data = s;
    let mut data = s;
    let mut char_data_index = 0;
    let mut out = Vec::<u8>::new();
    while let Some((char_i, (byte_i, _))) = data
        .code_point_indices()
        .enumerate()
        .find(|(_, (_, c))| !target_kind.can_encode(*c))
    {
        out.extend_from_slice(&data.as_bytes()[..byte_i]);
        let char_start = char_data_index + char_i;

        // number of non-compatible chars between the first non-compatible char and the next compatible char
        let non_compat_run_length = data[byte_i..]
            .code_points()
            .take_while(|c| !target_kind.can_encode(*c))
            .count();
        let char_range = char_start..char_start + non_compat_run_length;
        let (replace, char_restart) =
            errors.handle_encode_error(full_data, char_range.clone(), err_reason)?;
        match replace {
            EncodeReplace::Str(s) => {
                if s.is_compatible_with(target_kind) {
                    out.extend_from_slice(s.as_ref().as_bytes());
                } else {
                    return Err(errors.error_encoding(full_data, char_range, err_reason));
                }
            }
            EncodeReplace::Bytes(b) => {
                out.extend_from_slice(b.as_ref());
            }
        }
        data = crate::str::try_get_codepoints(full_data, char_restart..)
            .ok_or_else(|| errors.error_oob_restart(char_restart))?;
        char_data_index = char_restart;
    }
    out.extend_from_slice(data.as_bytes());
    Ok(out)
}

pub mod utf8 {
    use super::*;

    pub const ENCODING_NAME: &str = "utf-8";

    #[inline]
    pub fn encode<E: ErrorHandler>(s: &Wtf8, errors: &E) -> Result<Vec<u8>, E::Error> {
        encode_utf8_compatible(s, errors, "surrogates not allowed", StrKind::Utf8)
    }

    pub fn decode<E: ErrorHandler>(
        data: &[u8],
        errors: &E,
        final_decode: bool,
    ) -> Result<(Wtf8Buf, usize), E::Error> {
        decode_utf8_compatible(
            data,
            errors,
            |v| {
                core::str::from_utf8(v).map_err(|e| {
                    // SAFETY: as specified in valid_up_to's documentation, input[..e.valid_up_to()]
                    //         is valid utf8
                    unsafe { make_decode_err(v, e.valid_up_to(), e.error_len()) }
                })
            },
            |rest, err_len| {
                let first_err = rest[0];
                if matches!(first_err, 0x80..=0xc1 | 0xf5..=0xff) {
                    HandleResult::Error {
                        err_len: Some(1),
                        reason: "invalid start byte",
                    }
                } else if err_len.is_none() {
                    // error_len() == None means unexpected eof
                    if final_decode {
                        HandleResult::Error {
                            err_len,
                            reason: "unexpected end of data",
                        }
                    } else {
                        HandleResult::Done
                    }
                } else if !final_decode && matches!(rest, [0xed, 0xa0..=0xbf]) {
                    // truncated surrogate
                    HandleResult::Done
                } else {
                    HandleResult::Error {
                        err_len,
                        reason: "invalid continuation byte",
                    }
                }
            },
        )
    }
}

pub mod latin_1 {

    use super::*;

    pub const ENCODING_NAME: &str = "latin-1";

    const ERR_REASON: &str = "ordinal not in range(256)";

    #[inline]
    pub fn encode<E: ErrorHandler>(s: &Wtf8, errors: &E) -> Result<Vec<u8>, E::Error> {
        let full_data = s;
        let mut data = s;
        let mut char_data_index = 0;
        let mut out = Vec::<u8>::new();
        loop {
            match data
                .code_point_indices()
                .enumerate()
                .find(|(_, (_, c))| !c.is_ascii())
            {
                None => {
                    out.extend_from_slice(data.as_bytes());
                    break;
                }
                Some((char_i, (byte_i, ch))) => {
                    out.extend_from_slice(&data.as_bytes()[..byte_i]);
                    let char_start = char_data_index + char_i;
                    if let Some(byte) = ch.to_u32().to_u8() {
                        out.push(byte);
                        // if the codepoint is between 128..=255, it's utf8-length is 2
                        data = &data[byte_i + 2..];
                        char_data_index = char_start + 1;
                    } else {
                        // number of non-latin_1 chars between the first non-latin_1 char and the next latin_1 char
                        let non_latin_1_run_length = data[byte_i..]
                            .code_points()
                            .take_while(|c| c.to_u32() > 255)
                            .count();
                        let char_range = char_start..char_start + non_latin_1_run_length;
                        let (replace, char_restart) = errors.handle_encode_error(
                            full_data,
                            char_range.clone(),
                            ERR_REASON,
                        )?;
                        match replace {
                            EncodeReplace::Str(s) => {
                                if s.as_ref().code_points().any(|c| c.to_u32() > 255) {
                                    return Err(
                                        errors.error_encoding(full_data, char_range, ERR_REASON)
                                    );
                                }
                                out.extend_from_slice(s.as_ref().as_bytes());
                            }
                            EncodeReplace::Bytes(b) => {
                                out.extend_from_slice(b.as_ref());
                            }
                        }
                        data = crate::str::try_get_codepoints(full_data, char_restart..)
                            .ok_or_else(|| errors.error_oob_restart(char_restart))?;
                        char_data_index = char_restart;
                    }
                    continue;
                }
            }
        }
        Ok(out)
    }

    pub fn decode<E: ErrorHandler>(data: &[u8], _errors: &E) -> Result<(Wtf8Buf, usize), E::Error> {
        let out: String = data.iter().map(|c| *c as char).collect();
        let out_len = out.len();
        Ok((out.into(), out_len))
    }
}

pub mod ascii {
    use super::*;
    use ::ascii::AsciiStr;

    pub const ENCODING_NAME: &str = "ascii";

    const ERR_REASON: &str = "ordinal not in range(128)";

    #[inline]
    pub fn encode<E: ErrorHandler>(s: &Wtf8, errors: &E) -> Result<Vec<u8>, E::Error> {
        encode_utf8_compatible(s, errors, ERR_REASON, StrKind::Ascii)
    }

    pub fn decode<E: ErrorHandler>(data: &[u8], errors: &E) -> Result<(Wtf8Buf, usize), E::Error> {
        decode_utf8_compatible(
            data,
            errors,
            |v| {
                AsciiStr::from_ascii(v).map(|s| s.as_str()).map_err(|e| {
                    // SAFETY: as specified in valid_up_to's documentation, input[..e.valid_up_to()]
                    //         is valid ascii & therefore valid utf8
                    unsafe { make_decode_err(v, e.valid_up_to(), Some(1)) }
                })
            },
            |_rest, err_len| HandleResult::Error {
                err_len,
                reason: ERR_REASON,
            },
        )
    }
}
