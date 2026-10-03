// Scalar native equivalents of CPython 3.15 Modules/binascii.c base-N codecs.

pub(super) const BASE64: &[u8] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
pub(super) const BASE32: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
pub(super) const BASE85: &[u8] =
    b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz!#$%&()*+-;<=>?@^_`{|}~";
pub(super) const ASCII85: &[u8] =
    b"!\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstu";

#[derive(Debug)]
pub(super) enum Error {
    InvalidAlphabet(usize),
    Codec(String),
    Memory,
}

type Result<T> = core::result::Result<T, Error>;

fn codec(message: impl Into<String>) -> Error {
    Error::Codec(message.into())
}

fn buffer(size: usize, too_large: &str) -> Result<Vec<u8>> {
    if size > isize::MAX as usize {
        return Err(codec(too_large));
    }
    let mut result = Vec::new();
    result.try_reserve_exact(size).map_err(|_| Error::Memory)?;
    Ok(result)
}

fn checked_alphabet(alphabet: &[u8], size: usize) -> Result<&[u8]> {
    if alphabet.len() != size {
        return Err(Error::InvalidAlphabet(size));
    }
    Ok(alphabet)
}

pub(super) fn copy_alphabet(alphabet: &[u8], size: usize) -> Result<Vec<u8>> {
    checked_alphabet(alphabet, size)?;
    let mut out = buffer(size, "Too large alphabet")?;
    out.extend_from_slice(alphabet);
    Ok(out)
}

fn reverse_alphabet(alphabet: &[u8], size: usize, padded: bool) -> Result<[u8; 256]> {
    checked_alphabet(alphabet, size)?;
    let mut table = [u8::MAX; 256];
    for (digit, &ch) in alphabet.iter().enumerate() {
        // Duplicate characters are allowed; their last occurrence wins.
        table[usize::from(ch)] = digit as u8;
    }
    if padded {
        table[usize::from(b'=')] = size as u8;
    }
    Ok(table)
}

pub(super) fn ignored(ignorechars: &[u8]) -> [bool; 256] {
    let mut table = [false; 256];
    for &ch in ignorechars {
        table[usize::from(ch)] = true;
    }
    table
}

fn rounded_width(width: usize, quantum: usize) -> usize {
    if width == 0 {
        0
    } else {
        width.max(quantum) / quantum * quantum
    }
}

fn wrapped_size(size: usize, width: usize, newline: bool, too_large: &str) -> Result<usize> {
    let breaks = if width == 0 || size == 0 {
        0
    } else {
        (size - 1) / width
    };
    size.checked_add(breaks)
        .and_then(|n| n.checked_add(usize::from(newline)))
        .filter(|n| *n <= isize::MAX as usize)
        .ok_or_else(|| codec(too_large))
}

fn wrap(mut input: Vec<u8>, width: usize, newline: bool, too_large: &str) -> Result<Vec<u8>> {
    let size = wrapped_size(input.len(), width, newline, too_large)?;
    if width != 0 && input.len() > width {
        let mut out = buffer(size, too_large)?;
        for (index, chunk) in input.chunks(width).enumerate() {
            if index != 0 {
                out.push(b'\n');
            }
            out.extend_from_slice(chunk);
        }
        input = out;
    }
    if newline {
        input.try_reserve(1).map_err(|_| Error::Memory)?;
        input.push(b'\n');
    }
    Ok(input)
}

pub(super) fn encode_bits(
    data: &[u8],
    bits: u32,
    padded: bool,
    width: usize,
    newline: bool,
    alphabet: &[u8],
) -> Result<Vec<u8>> {
    let (size, in_group, out_group, too_large) = if bits == 6 {
        (64, 3, 4, "Too much data for base64")
    } else {
        (32, 5, 8, "Too much data for base32")
    };
    checked_alphabet(alphabet, size)?;
    let full_size = data
        .len()
        .div_ceil(in_group)
        .checked_mul(out_group)
        .ok_or_else(|| codec(too_large))?;
    let pads = (in_group - data.len() % in_group) % in_group * out_group / in_group;
    let encoded_size = full_size - if padded { 0 } else { pads };
    let width = rounded_width(width, out_group);
    wrapped_size(encoded_size, width, newline, too_large)?;
    if bits == 6 && alphabet == BASE64 {
        let mut out = rustpython_common::binascii::b2a_base64(data, false);
        out.truncate(encoded_size);
        return wrap(out, width, newline, too_large);
    }
    let mut out = buffer(encoded_size, too_large)?;
    let mut value = 0u32;
    let mut available = 0u32;
    let mask = (1u32 << bits) - 1;
    for &byte in data {
        value = (value << 8) | u32::from(byte);
        available += 8;
        while available >= bits {
            available -= bits;
            out.push(alphabet[((value >> available) & mask) as usize]);
        }
        value &= (1u32 << available) - 1;
    }
    if available != 0 {
        out.push(alphabet[((value << (bits - available)) & mask) as usize]);
    }
    if padded {
        out.resize(encoded_size, b'=');
    }
    wrap(out, width, newline, too_large)
}

pub(super) struct BitDecodeOptions {
    pub bits: u32,
    pub padded: bool,
    pub strict: bool,
    pub canonical: bool,
}

pub(super) fn decode_bits(
    data: &[u8],
    alphabet: &[u8],
    ignorechars: &[bool; 256],
    options: BitDecodeOptions,
) -> Result<Vec<u8>> {
    let BitDecodeOptions {
        bits,
        padded,
        strict,
        canonical,
    } = options;
    let (size, group, decoded_group, name) = if bits == 6 {
        (64, 4, 3, "base64")
    } else {
        (32, 8, 5, "base32")
    };
    // '=' is reserved even when input padding is disabled.
    let table = reverse_alphabet(alphabet, size, true)?;
    let ignore = ignorechars;
    let capacity = data
        .len()
        .div_ceil(group)
        .checked_mul(decoded_group)
        .ok_or_else(|| codec(format!("Too much {name} data")))?;
    let mut out = buffer(capacity, &format!("Too much {name} data"))?;
    let mut pos = 0usize;
    let mut pads = 0usize;
    let mut value = 0u32;
    let mut available = 0u32;
    for &ch in data {
        if padded && ch == b'=' {
            pads += 1;
            let allowed_pos = if bits == 6 {
                pos >= 2
            } else {
                matches!(pos, 2 | 4 | 5 | 7)
            };
            if allowed_pos && pos + pads <= group {
                continue;
            }
            if !strict || ignore[usize::from(b'=')] {
                continue;
            }
            if pos == 1 || (bits == 5 && matches!(pos, 3 | 6)) {
                break;
            }
            return Err(codec(if pos == 0 && out.is_empty() {
                "Leading padding not allowed"
            } else {
                "Excess padding not allowed"
            }));
        }
        let digit = table[usize::from(ch)];
        if usize::from(digit) >= size {
            if strict && !ignore[usize::from(ch)] {
                return Err(codec(if ch == b'=' {
                    "Padding not allowed".to_owned()
                } else {
                    format!("Only {name} data is allowed")
                }));
            }
            continue;
        }
        if pads != 0 && strict && !ignore[usize::from(b'=')] {
            return Err(codec(if pos + pads == group {
                "Excess data after padding"
            } else {
                "Discontinuous padding not allowed"
            }));
        }
        if bits == 6 {
            pads = 0;
        }
        value = (value << bits) | u32::from(digit);
        available += bits;
        if available >= 8 {
            available -= 8;
            out.push((value >> available) as u8);
            value &= (1u32 << available) - 1;
        }
        pos += 1;
        if pos == group {
            pos = 0;
            pads = 0;
        }
    }
    if pos == 1 || (bits == 5 && matches!(pos, 3 | 6)) {
        let count = out.len() / decoded_group * group + pos;
        return Err(codec(if bits == 6 {
            format!(
                "Invalid base64-encoded string: number of data characters ({count}) cannot be 1 more than a multiple of 4"
            )
        } else {
            format!(
                "Invalid base32-encoded string: number of data characters ({count}) cannot be 1, 3, or 6 more than a multiple of 8"
            )
        }));
    }
    if padded && pos != 0 && pos + pads < group {
        return Err(codec("Incorrect padding"));
    }
    if canonical && value != 0 {
        return Err(codec("Non-zero padding bits"));
    }
    Ok(out)
}

fn digits85(mut value: u32, alphabet: &[u8]) -> [u8; 5] {
    let mut out = [0; 5];
    for slot in out.iter_mut().rev() {
        *slot = alphabet[(value % 85) as usize];
        value /= 85;
    }
    out
}

pub(super) fn encode85(
    data: &[u8],
    pad: bool,
    width: usize,
    alphabet: &[u8],
    ascii85: bool,
    foldspaces: bool,
    adobe: bool,
) -> Result<Vec<u8>> {
    checked_alphabet(alphabet, 85)?;
    let too_large = if ascii85 {
        "Too much data for Ascii85"
    } else {
        "Too much data for Base85"
    };
    let full_size = data
        .len()
        .div_ceil(4)
        .checked_mul(5)
        .and_then(|n| n.checked_add(if adobe { 4 } else { 0 }))
        .ok_or_else(|| codec(too_large))?;
    let size = full_size
        - if !pad && data.len() % 4 != 0 {
            4 - data.len() % 4
        } else {
            0
        };
    let width = if ascii85 {
        if adobe && width == 1 { 2 } else { width }
    } else {
        rounded_width(width, 5)
    };
    wrapped_size(size, width, false, too_large)?;
    let mut out = buffer(size, too_large)?;
    if adobe {
        out.extend_from_slice(b"<~");
    }
    for chunk in data.chunks(4) {
        let mut block = [0; 4];
        block[..chunk.len()].copy_from_slice(chunk);
        let value = u32::from_be_bytes(block);
        if ascii85 && value == 0 && (pad || chunk.len() == 4) {
            out.push(b'z');
        } else if ascii85 && foldspaces && value == 0x20202020 && chunk.len() == 4 {
            out.push(b'y');
        } else {
            let digits = digits85(value, alphabet);
            let count = if pad || chunk.len() == 4 {
                5
            } else {
                chunk.len() + 1
            };
            out.extend_from_slice(&digits[..count]);
        }
    }
    if adobe {
        out.extend_from_slice(b"~>");
    }
    let mut out = wrap(out, width, false, too_large)?;
    if adobe && out.len() >= 3 && out[out.len() - 2] == b'\n' {
        let len = out.len();
        out[len - 3] = b'\n';
        out[len - 2] = b'~';
    }
    Ok(out)
}

pub(super) fn decode85(
    mut data: &[u8],
    alphabet: &[u8],
    ignorechars: &[bool; 256],
    canonical: bool,
    ascii85: bool,
    foldspaces: bool,
    adobe: bool,
) -> Result<Vec<u8>> {
    let table = reverse_alphabet(alphabet, 85, false)?;
    if adobe {
        if !data.ends_with(b"~>") {
            return Err(codec("Ascii85 encoded byte sequences must end with b'~>'"));
        }
        data = &data[..data.len() - 2];
        if data.starts_with(b"<~") {
            data = &data[2..];
        }
    }
    let ignore = ignorechars;
    let shortcuts = if ascii85 {
        data.iter().filter(|ch| matches!(ch, b'y' | b'z')).count()
    } else {
        0
    };
    let size = (data.len() - shortcuts)
        .div_ceil(5)
        .checked_mul(4)
        .and_then(|n| shortcuts.checked_mul(4).and_then(|m| n.checked_add(m)))
        .ok_or_else(|| codec("Too much Ascii85 data"))?;
    let mut out = buffer(size, "Too much Ascii85 data")?;
    let mut value = 0u32;
    let mut pos = 0usize;
    let mut from_z = false;
    let mut index = 0usize;
    let mut tail_len = 4usize;
    while index < data.len() || pos != 0 {
        let (ch, digit) = if index < data.len() {
            let ch = data[index];
            (ch, table[usize::from(ch)])
        } else {
            tail_len -= 1;
            (0, 84)
        };
        if digit < 85 {
            value = value
                .checked_mul(85)
                .and_then(|n| n.checked_add(u32::from(digit)))
                .ok_or_else(|| {
                    codec(if ascii85 {
                        "Ascii85 overflow".to_owned()
                    } else {
                        format!("Base85 overflow in hunk starting at byte {}", index / 5 * 5)
                    })
                })?;
            pos += 1;
        } else if ascii85 && (ch == b'z' || (ch == b'y' && foldspaces)) {
            if pos != 0 {
                return Err(codec(format!(
                    "'{}' inside Ascii85 5-tuple",
                    char::from(ch)
                )));
            }
            value = if ch == b'y' { 0x20202020 } else { 0 };
            from_z = ch == b'z';
            pos = 5;
        } else if !ignore[usize::from(ch)] {
            return Err(codec(if ascii85 {
                format!("Non-Ascii85 digit found: {}", char::from(ch))
            } else {
                format!("bad Base85 character at position {index}")
            }));
        }
        index += 1;
        if pos != 5 {
            continue;
        }
        if tail_len == 0 {
            return Err(codec(if ascii85 {
                "Incomplete Ascii85 group"
            } else {
                "Incomplete Base85 group"
            }));
        }
        out.extend_from_slice(&value.to_be_bytes()[..tail_len]);
        if canonical {
            if ascii85 && tail_len == 4 && value == 0 && !from_z {
                return Err(codec("Non-canonical encoding, use 'z' for all-zero groups"));
            }
            if tail_len < 4 {
                let pads = 4 - tail_len;
                let top = value >> (pads * 8) << (pads * 8);
                let divisor = 85u32.pow(pads as u32);
                if top / divisor != value / divisor {
                    return Err(codec("Non-zero padding bits"));
                }
            }
        }
        pos = 0;
        value = 0;
        from_z = false;
    }
    Ok(out)
}

pub(super) fn decode_hex(data: &[u8], ignorechars: &[bool; 256]) -> Result<Vec<u8>> {
    let ignore = ignorechars;
    let mut out = buffer(data.len() / 2, "Too much hexadecimal data")?;
    let mut high = None;
    for &ch in data {
        let digit = match ch {
            b'0'..=b'9' => ch - b'0',
            b'a'..=b'f' => ch - b'a' + 10,
            b'A'..=b'F' => ch - b'A' + 10,
            _ if ignore[usize::from(ch)] => continue,
            _ => return Err(codec("Non-hexadecimal digit found")),
        };
        if let Some(hi) = high.take() {
            out.push((hi << 4) | digit);
        } else {
            high = Some(digit);
        }
    }
    if high.is_some() {
        return Err(codec("Odd number of hexadecimal digits"));
    }
    Ok(out)
}
