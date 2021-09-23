use std::ops::Range;

pub type EncodeErrorResult<S, B, E> = Result<(EncodeReplace<S, B>, usize), E>;

pub type DecodeErrorResult<S, B, E> = Result<(S, Option<B>, usize), E>;

pub trait StrBuffer: AsRef<str> {
    fn is_ascii(&self) -> bool {
        self.as_ref().is_ascii()
    }
}

pub trait ErrorHandler {
    type Error;
    type StrBuf: StrBuffer;
    type BytesBuf: AsRef<[u8]>;
    fn handle_encode_error(
        &self,
        data: &str,
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
    fn error_encoding(&self, data: &str, char_range: Range<usize>, reason: &str) -> Self::Error;
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
    let valid_prefix = core::str::from_utf8_unchecked(v.get_unchecked(..valid_up_to));
    let rest = v.get_unchecked(valid_up_to..);
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
) -> Result<(String, usize), E::Error>
where
    DecodeF: Fn(&[u8]) -> Result<&str, DecodeError<'_>>,
    ErrF: Fn(&[u8], Option<usize>) -> HandleResult<'_>,
{
    if data.is_empty() {
        return Ok((String::new(), 0));
    }
    // we need to coerce the lifetime to that of the function body rather than the
    // anonymous input lifetime, so that we can assign it data borrowed from data_from_err
    let mut data = &*data;
    let mut data_from_err: E::BytesBuf;
    let mut out = String::with_capacity(data.len());
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
                        out.push_str(replace.as_ref());
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

pub mod utf8 {
    use super::*;

    pub const ENCODING_NAME: &str = "utf-8";

    #[inline]
    pub fn encode<E: ErrorHandler>(s: &str, _errors: &E) -> Result<Vec<u8>, E::Error> {
        Ok(s.as_bytes().to_vec())
    }

    pub fn decode<E: ErrorHandler>(
        data: &[u8],
        errors: &E,
        final_decode: bool,
    ) -> Result<(String, usize), E::Error> {
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

pub mod ascii {
    use super::*;
    use ::ascii::AsciiStr;

    pub const ENCODING_NAME: &str = "ascii";

    const ERR_REASON: &str = "ordinal not in range(128)";

    #[inline]
    pub fn encode<E: ErrorHandler>(s: &str, errors: &E) -> Result<Vec<u8>, E::Error> {
        let full_data = s;
        let mut data = s;
        let mut char_data_index = 0;
        let mut out = Vec::<u8>::new();
        loop {
            match data
                .char_indices()
                .enumerate()
                .find(|(_, (_, c))| !c.is_ascii())
            {
                None => {
                    out.extend_from_slice(data.as_bytes());
                    break;
                }
                Some((char_i, (byte_i, _))) => {
                    out.extend_from_slice(&data.as_bytes()[..byte_i]);
                    let char_start = char_data_index + char_i;
                    // number of non-ascii chars between the first non-ascii char and the next ascii char
                    let non_ascii_run_length =
                        data[byte_i..].chars().take_while(|c| !c.is_ascii()).count();
                    let char_range = char_start..char_start + non_ascii_run_length;
                    let (replace, char_restart) =
                        errors.handle_encode_error(full_data, char_range.clone(), ERR_REASON)?;
                    match replace {
                        EncodeReplace::Str(s) => {
                            if !s.is_ascii() {
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
                    data = crate::str::try_get_chars(full_data, char_restart..)
                        .ok_or_else(|| errors.error_oob_restart(char_restart))?;
                    char_data_index = char_restart;
                    continue;
                }
            }
        }
        Ok(out)
    }

    pub fn decode<E: ErrorHandler>(data: &[u8], errors: &E) -> Result<(String, usize), E::Error> {
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
