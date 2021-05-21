use std::ops::Range;

pub type EncodeErrorResult<S, B, E> = Result<(EncodeReplace<S, B>, usize), E>;

pub type DecodeErrorResult<S, B, E> = Result<(S, Option<B>, usize), E>;

pub trait ErrorHandler {
    type Error;
    type StrBuf: AsRef<str>;
    type BytesBuf: AsRef<[u8]>;
    fn handle_encode_error(
        &self,
        byte_range: Range<usize>,
        reason: &str,
    ) -> EncodeErrorResult<Self::StrBuf, Self::BytesBuf, Self::Error>;
    fn handle_decode_error(
        &self,
        data: &[u8],
        byte_range: Range<usize>,
        reason: &str,
    ) -> DecodeErrorResult<Self::StrBuf, Self::BytesBuf, Self::Error>;
    fn error_oob_restart(&self, i: usize) -> Self::Error;
}
pub enum EncodeReplace<S, B> {
    Str(S),
    Bytes(B),
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
        macro_rules! handle_error {
            ($range:expr, $reason:expr) => {{
                let (replace, new_data, restart) =
                    errors.handle_decode_error(data, $range, $reason)?;
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
            }};
        }
        loop {
            match core::str::from_utf8(remaining_data) {
                Ok(decoded) => {
                    out.push_str(decoded);
                    remaining_index += decoded.len();
                    break;
                }
                Err(e) => {
                    let (valid_prefix, rest, first_err) = unsafe {
                        let index = e.valid_up_to();
                        // SAFETY: as specified in valid_up_to's documentation, from_utf8(&input[..index]) will return Ok(_)
                        let valid =
                            std::str::from_utf8_unchecked(remaining_data.get_unchecked(..index));
                        let rest = remaining_data.get_unchecked(index..);
                        // SAFETY: if index didn't have something at it, this wouldn't be an error
                        let first_err = *remaining_data.get_unchecked(index);
                        (valid, rest, first_err)
                    };
                    out.push_str(valid_prefix);
                    let err_idx = remaining_index + e.valid_up_to();
                    remaining_data = rest;
                    remaining_index += valid_prefix.len();
                    if (0x80..0xc0).contains(&first_err) {
                        handle_error!(err_idx..err_idx + 1, "invalid start byte");
                    }
                    let err_len = match e.error_len() {
                        Some(l) => l,
                        // error_len() == None means unexpected eof
                        None => {
                            if !final_decode {
                                break;
                            }
                            handle_error!(err_idx..data.len(), "unexpected end of data");
                        }
                    };
                    if !final_decode && matches!(remaining_data, [0xed, 0xa0..=0xbf]) {
                        // truncated surrogate
                        break;
                    }
                    handle_error!(err_idx..err_idx + err_len, "invalid continuation byte");
                }
            }
        }
        Ok((out, remaining_index))
    }
}
