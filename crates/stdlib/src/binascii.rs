// spell-checker:ignore hexlify unhexlify uuencodes CRCTAB rlecode rledecode

pub(super) use decl::crc32;
pub(crate) use decl::make_module;
use rustpython_vm::{VirtualMachine, builtins::PyBaseExceptionRef, convert::ToPyException};

const PAD: u8 = 61u8;
const MAXLINESIZE: usize = 76; // Excluding the CRLF

#[pymodule(name = "binascii")]
mod decl {
    use super::{MAXLINESIZE, PAD};
    use crate::vm::{
        PyResult, VirtualMachine,
        builtins::{PyIntRef, PyTypeRef},
        convert::ToPyException,
        function::{ArgAsciiBuffer, ArgBytesLike, OptionalArg},
    };
    use base64::Engine;
    use itertools::Itertools;

    #[pyattr(name = "Error", once)]
    pub(super) fn error_type(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.new_exception_type(
            "binascii",
            "Error",
            Some(vec![vm.ctx.exceptions.value_error.to_owned()]),
        )
    }

    #[pyattr(name = "Incomplete", once)]
    fn incomplete_type(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.new_exception_type("binascii", "Incomplete", None)
    }

    fn hex_nibble(n: u8) -> u8 {
        match n {
            0..=9 => b'0' + n,
            10..=15 => b'a' + (n - 10),
            _ => unreachable!(),
        }
    }

    #[pyfunction(name = "b2a_hex")]
    #[pyfunction]
    fn hexlify(
        data: ArgBytesLike,
        sep: OptionalArg<ArgAsciiBuffer>,
        bytes_per_sep: OptionalArg<isize>,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<u8>> {
        let bytes_per_sep = bytes_per_sep.unwrap_or(1);

        data.with_ref(|bytes| {
            // Get separator character if provided
            let sep_char = if let OptionalArg::Present(sep_buf) = sep {
                sep_buf.with_ref(|sep_bytes| {
                    if sep_bytes.len() != 1 {
                        return Err(vm.new_value_error("sep must be length 1."));
                    }
                    let sep_char = sep_bytes[0];
                    if !sep_char.is_ascii() {
                        return Err(vm.new_value_error("sep must be ASCII."));
                    }
                    Ok(Some(sep_char))
                })?
            } else {
                None
            };

            // If no separator or bytes_per_sep is 0, use simple hexlify
            if sep_char.is_none() || bytes_per_sep == 0 || bytes.is_empty() {
                let mut hex = Vec::<u8>::with_capacity(bytes.len() * 2);
                for b in bytes {
                    hex.push(hex_nibble(b >> 4));
                    hex.push(hex_nibble(b & 0xf));
                }
                return Ok(hex);
            }

            let sep_char = sep_char.unwrap();
            let abs_bytes_per_sep = bytes_per_sep.unsigned_abs();

            // If separator interval is >= data length, no separators needed
            if abs_bytes_per_sep >= bytes.len() {
                let mut hex = Vec::<u8>::with_capacity(bytes.len() * 2);
                for b in bytes {
                    hex.push(hex_nibble(b >> 4));
                    hex.push(hex_nibble(b & 0xf));
                }
                return Ok(hex);
            }

            // Calculate result length
            let num_separators = (bytes.len() - 1) / abs_bytes_per_sep;
            let result_len = bytes.len() * 2 + num_separators;
            let mut hex = vec![0u8; result_len];

            if bytes_per_sep < 0 {
                // Left-to-right processing (negative bytes_per_sep)
                let mut i = 0; // input index
                let mut j = 0; // output index
                let chunks = bytes.len() / abs_bytes_per_sep;

                // Process complete chunks
                for _ in 0..chunks {
                    for _ in 0..abs_bytes_per_sep {
                        let b = bytes[i];
                        hex[j] = hex_nibble(b >> 4);
                        hex[j + 1] = hex_nibble(b & 0xf);
                        i += 1;
                        j += 2;
                    }
                    if i < bytes.len() {
                        hex[j] = sep_char;
                        j += 1;
                    }
                }

                // Process remaining bytes
                while i < bytes.len() {
                    let b = bytes[i];
                    hex[j] = hex_nibble(b >> 4);
                    hex[j + 1] = hex_nibble(b & 0xf);
                    i += 1;
                    j += 2;
                }
            } else {
                // Right-to-left processing (positive bytes_per_sep)
                let mut i = bytes.len() as isize - 1; // input index
                let mut j = result_len as isize - 1; // output index
                let chunks = bytes.len() / abs_bytes_per_sep;

                // Process complete chunks from right
                for _ in 0..chunks {
                    for _ in 0..abs_bytes_per_sep {
                        let b = bytes[i as usize];
                        hex[j as usize] = hex_nibble(b & 0xf);
                        hex[(j - 1) as usize] = hex_nibble(b >> 4);
                        i -= 1;
                        j -= 2;
                    }
                    if i >= 0 {
                        hex[j as usize] = sep_char;
                        j -= 1;
                    }
                }

                // Process remaining bytes
                while i >= 0 {
                    let b = bytes[i as usize];
                    hex[j as usize] = hex_nibble(b & 0xf);
                    hex[(j - 1) as usize] = hex_nibble(b >> 4);
                    i -= 1;
                    j -= 2;
                }
            }

            Ok(hex)
        })
    }

    const fn unhex_nibble(c: u8) -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    }

    #[pyfunction(name = "a2b_hex")]
    #[pyfunction]
    fn unhexlify(data: ArgAsciiBuffer, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        data.with_ref(|hex_bytes| {
            if hex_bytes.len() % 2 != 0 {
                return Err(super::new_binascii_error(
                    "Odd-length string".to_owned(),
                    vm,
                ));
            }

            let mut unhex = Vec::<u8>::with_capacity(hex_bytes.len() / 2);
            for (n1, n2) in hex_bytes.iter().tuples() {
                if let (Some(n1), Some(n2)) = (unhex_nibble(*n1), unhex_nibble(*n2)) {
                    unhex.push((n1 << 4) | n2);
                } else {
                    return Err(super::new_binascii_error(
                        "Non-hexadecimal digit found".to_owned(),
                        vm,
                    ));
                }
            }

            Ok(unhex)
        })
    }

    #[pyfunction]
    pub(crate) fn crc32(data: ArgBytesLike, init: OptionalArg<PyIntRef>) -> u32 {
        let init = init.map_or(0, |i| i.as_u32_mask());

        let mut hasher = crc32fast::Hasher::new_with_initial(init);
        data.with_ref(|bytes| {
            hasher.update(bytes);
            hasher.finalize()
        })
    }

    #[pyfunction]
    pub(crate) fn crc_hqx(data: ArgBytesLike, init: PyIntRef) -> u32 {
        const CRCTAB_HQX: [u16; 256] = [
            0x0000, 0x1021, 0x2042, 0x3063, 0x4084, 0x50a5, 0x60c6, 0x70e7, 0x8108, 0x9129, 0xa14a,
            0xb16b, 0xc18c, 0xd1ad, 0xe1ce, 0xf1ef, 0x1231, 0x0210, 0x3273, 0x2252, 0x52b5, 0x4294,
            0x72f7, 0x62d6, 0x9339, 0x8318, 0xb37b, 0xa35a, 0xd3bd, 0xc39c, 0xf3ff, 0xe3de, 0x2462,
            0x3443, 0x0420, 0x1401, 0x64e6, 0x74c7, 0x44a4, 0x5485, 0xa56a, 0xb54b, 0x8528, 0x9509,
            0xe5ee, 0xf5cf, 0xc5ac, 0xd58d, 0x3653, 0x2672, 0x1611, 0x0630, 0x76d7, 0x66f6, 0x5695,
            0x46b4, 0xb75b, 0xa77a, 0x9719, 0x8738, 0xf7df, 0xe7fe, 0xd79d, 0xc7bc, 0x48c4, 0x58e5,
            0x6886, 0x78a7, 0x0840, 0x1861, 0x2802, 0x3823, 0xc9cc, 0xd9ed, 0xe98e, 0xf9af, 0x8948,
            0x9969, 0xa90a, 0xb92b, 0x5af5, 0x4ad4, 0x7ab7, 0x6a96, 0x1a71, 0x0a50, 0x3a33, 0x2a12,
            0xdbfd, 0xcbdc, 0xfbbf, 0xeb9e, 0x9b79, 0x8b58, 0xbb3b, 0xab1a, 0x6ca6, 0x7c87, 0x4ce4,
            0x5cc5, 0x2c22, 0x3c03, 0x0c60, 0x1c41, 0xedae, 0xfd8f, 0xcdec, 0xddcd, 0xad2a, 0xbd0b,
            0x8d68, 0x9d49, 0x7e97, 0x6eb6, 0x5ed5, 0x4ef4, 0x3e13, 0x2e32, 0x1e51, 0x0e70, 0xff9f,
            0xefbe, 0xdfdd, 0xcffc, 0xbf1b, 0xaf3a, 0x9f59, 0x8f78, 0x9188, 0x81a9, 0xb1ca, 0xa1eb,
            0xd10c, 0xc12d, 0xf14e, 0xe16f, 0x1080, 0x00a1, 0x30c2, 0x20e3, 0x5004, 0x4025, 0x7046,
            0x6067, 0x83b9, 0x9398, 0xa3fb, 0xb3da, 0xc33d, 0xd31c, 0xe37f, 0xf35e, 0x02b1, 0x1290,
            0x22f3, 0x32d2, 0x4235, 0x5214, 0x6277, 0x7256, 0xb5ea, 0xa5cb, 0x95a8, 0x8589, 0xf56e,
            0xe54f, 0xd52c, 0xc50d, 0x34e2, 0x24c3, 0x14a0, 0x0481, 0x7466, 0x6447, 0x5424, 0x4405,
            0xa7db, 0xb7fa, 0x8799, 0x97b8, 0xe75f, 0xf77e, 0xc71d, 0xd73c, 0x26d3, 0x36f2, 0x0691,
            0x16b0, 0x6657, 0x7676, 0x4615, 0x5634, 0xd94c, 0xc96d, 0xf90e, 0xe92f, 0x99c8, 0x89e9,
            0xb98a, 0xa9ab, 0x5844, 0x4865, 0x7806, 0x6827, 0x18c0, 0x08e1, 0x3882, 0x28a3, 0xcb7d,
            0xdb5c, 0xeb3f, 0xfb1e, 0x8bf9, 0x9bd8, 0xabbb, 0xbb9a, 0x4a75, 0x5a54, 0x6a37, 0x7a16,
            0x0af1, 0x1ad0, 0x2ab3, 0x3a92, 0xfd2e, 0xed0f, 0xdd6c, 0xcd4d, 0xbdaa, 0xad8b, 0x9de8,
            0x8dc9, 0x7c26, 0x6c07, 0x5c64, 0x4c45, 0x3ca2, 0x2c83, 0x1ce0, 0x0cc1, 0xef1f, 0xff3e,
            0xcf5d, 0xdf7c, 0xaf9b, 0xbfba, 0x8fd9, 0x9ff8, 0x6e17, 0x7e36, 0x4e55, 0x5e74, 0x2e93,
            0x3eb2, 0x0ed1, 0x1ef0,
        ];

        let mut crc = init.as_u32_mask() & 0xffff;

        data.with_ref(|buf| {
            for byte in buf {
                crc =
                    ((crc << 8) & 0xFF00) ^ CRCTAB_HQX[((crc >> 8) as u8 ^ (byte)) as usize] as u32;
            }
        });

        crc
    }

    #[derive(FromArgs)]
    struct NewlineArg {
        #[pyarg(named, default = true)]
        newline: bool,
    }

    #[derive(FromArgs)]
    struct A2bBase64Args {
        #[pyarg(any)]
        s: ArgAsciiBuffer,
        #[pyarg(named, default = false)]
        strict_mode: bool,
    }

    #[pyfunction]
    fn a2b_base64(args: A2bBase64Args, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        #[rustfmt::skip]
        // Converts between ASCII and base-64 characters. The index of a given number yields the
        // number in ASCII while the value of said index yields the number in base-64. For example
        // "=" is 61 in ASCII but 0 (since it's the pad character) in base-64, so BASE64_TABLE[61] == 0
        const BASE64_TABLE: [i8; 256] = [
            -1,-1,-1,-1, -1,-1,-1,-1, -1,-1,-1,-1, -1,-1,-1,-1,
            -1,-1,-1,-1, -1,-1,-1,-1, -1,-1,-1,-1, -1,-1,-1,-1,
            -1,-1,-1,-1, -1,-1,-1,-1, -1,-1,-1,62, -1,-1,-1,63,
            52,53,54,55, 56,57,58,59, 60,61,-1,-1, -1, 0,-1,-1, /* Note PAD->0 */
            -1, 0, 1, 2,  3, 4, 5, 6,  7, 8, 9,10, 11,12,13,14,
            15,16,17,18, 19,20,21,22, 23,24,25,-1, -1,-1,-1,-1,
            -1,26,27,28, 29,30,31,32, 33,34,35,36, 37,38,39,40,
            41,42,43,44, 45,46,47,48, 49,50,51,-1, -1,-1,-1,-1,

            -1,-1,-1,-1, -1,-1,-1,-1, -1,-1,-1,-1, -1,-1,-1,-1,
            -1,-1,-1,-1, -1,-1,-1,-1, -1,-1,-1,-1, -1,-1,-1,-1,
            -1,-1,-1,-1, -1,-1,-1,-1, -1,-1,-1,-1, -1,-1,-1,-1,
            -1,-1,-1,-1, -1,-1,-1,-1, -1,-1,-1,-1, -1,-1,-1,-1,
            -1,-1,-1,-1, -1,-1,-1,-1, -1,-1,-1,-1, -1,-1,-1,-1,
            -1,-1,-1,-1, -1,-1,-1,-1, -1,-1,-1,-1, -1,-1,-1,-1,
            -1,-1,-1,-1, -1,-1,-1,-1, -1,-1,-1,-1, -1,-1,-1,-1,
            -1,-1,-1,-1, -1,-1,-1,-1, -1,-1,-1,-1, -1,-1,-1,-1,
        ];

        let A2bBase64Args { s, strict_mode } = args;
        s.with_ref(|b| {
            if b.is_empty() {
                return Ok(vec![]);
            }

            if strict_mode && b[0] == PAD {
                return Err(base64::DecodeError::InvalidByte(0, 61));
            }

            let mut decoded: Vec<u8> = vec![];

            let mut quad_pos = 0; // position in the nibble
            let mut pads = 0;
            let mut left_char: u8 = 0;
            let mut padding_started = false;
            for (i, &el) in b.iter().enumerate() {
                if el == PAD {
                    padding_started = true;

                    pads += 1;
                    if quad_pos >= 2 && quad_pos + pads >= 4 {
                        if strict_mode && i + 1 < b.len() {
                            // Represents excess data after padding error
                            return Err(base64::DecodeError::InvalidLastSymbol(i, PAD));
                        }

                        return Ok(decoded);
                    }

                    continue;
                }

                let binary_char = BASE64_TABLE[el as usize];
                if binary_char >= 64 || binary_char == -1 {
                    if strict_mode {
                        // Represents non-base64 data error
                        return Err(base64::DecodeError::InvalidByte(i, el));
                    }
                    continue;
                }

                if strict_mode && padding_started {
                    // Represents discontinuous padding error
                    return Err(base64::DecodeError::InvalidByte(i, PAD));
                }
                pads = 0;

                // Decode individual ASCII character
                match quad_pos {
                    0 => {
                        quad_pos = 1;
                        left_char = binary_char as u8;
                    }
                    1 => {
                        quad_pos = 2;
                        decoded.push((left_char << 2) | (binary_char >> 4) as u8);
                        left_char = (binary_char & 0x0f) as u8;
                    }
                    2 => {
                        quad_pos = 3;
                        decoded.push((left_char << 4) | (binary_char >> 2) as u8);
                        left_char = (binary_char & 0x03) as u8;
                    }
                    3 => {
                        quad_pos = 0;
                        decoded.push((left_char << 6) | binary_char as u8);
                        left_char = 0;
                    }
                    _ => unsafe {
                        // quad_pos is only assigned in this match statement to constants
                        core::hint::unreachable_unchecked()
                    },
                }
            }

            match quad_pos {
                0 => Ok(decoded),
                1 => Err(base64::DecodeError::InvalidLastSymbol(
                    decoded.len() / 3 * 4 + 1,
                    0,
                )),
                _ => Err(base64::DecodeError::InvalidLength(quad_pos)),
            }
        })
        .map_err(|err| super::Base64DecodeError(err).to_pyexception(vm))
    }

    #[pyfunction]
    fn b2a_base64(data: ArgBytesLike, NewlineArg { newline }: NewlineArg) -> Vec<u8> {
        // https://stackoverflow.com/questions/63916821
        let mut encoded = data
            .with_ref(|b| base64::engine::general_purpose::STANDARD.encode(b))
            .into_bytes();
        if newline {
            encoded.push(b'\n');
        }
        encoded
    }

    #[inline]
    fn uu_a2b_read(c: &u8, vm: &VirtualMachine) -> PyResult<u8> {
        // Check the character for legality
        // The 64 instead of the expected 63 is because
        // there are a few uuencodes out there that use
        // '`' as zero instead of space.
        if !(b' '..=(b' ' + 64)).contains(c) {
            if [b'\r', b'\n'].contains(c) {
                return Ok(0);
            }
            return Err(super::new_binascii_error("Illegal char".to_owned(), vm));
        }
        Ok((*c - b' ') & 0x3f)
    }

    #[derive(FromArgs)]
    struct A2bQpArgs {
        #[pyarg(any)]
        data: ArgAsciiBuffer,
        #[pyarg(named, default = false)]
        header: bool,
    }
    #[pyfunction]
    fn a2b_qp(args: A2bQpArgs) -> PyResult<Vec<u8>> {
        let s = args.data;
        let header = args.header;
        s.with_ref(|buffer| {
            let len = buffer.len();
            let mut out_data = Vec::with_capacity(len);

            let mut idx = 0;

            while idx < len {
                if buffer[idx] == b'=' {
                    idx += 1;
                    if idx >= len {
                        break;
                    }
                    // Soft line breaks
                    if (buffer[idx] == b'\n') || (buffer[idx] == b'\r') {
                        if buffer[idx] != b'\n' {
                            while idx < len && buffer[idx] != b'\n' {
                                idx += 1;
                            }
                        }
                        if idx < len {
                            idx += 1;
                        }
                    } else if buffer[idx] == b'=' {
                        // broken case from broken python qp
                        out_data.push(b'=');
                        idx += 1;
                    } else if idx + 1 < len
                        && ((buffer[idx] >= b'A' && buffer[idx] <= b'F')
                            || (buffer[idx] >= b'a' && buffer[idx] <= b'f')
                            || (buffer[idx] >= b'0' && buffer[idx] <= b'9'))
                        && ((buffer[idx + 1] >= b'A' && buffer[idx + 1] <= b'F')
                            || (buffer[idx + 1] >= b'a' && buffer[idx + 1] <= b'f')
                            || (buffer[idx + 1] >= b'0' && buffer[idx + 1] <= b'9'))
                    {
                        // hex val
                        if let (Some(ch1), Some(ch2)) =
                            (unhex_nibble(buffer[idx]), unhex_nibble(buffer[idx + 1]))
                        {
                            out_data.push((ch1 << 4) | ch2);
                        }
                        idx += 2;
                    } else {
                        out_data.push(b'=');
                    }
                } else if header && buffer[idx] == b'_' {
                    out_data.push(b' ');
                    idx += 1;
                } else {
                    out_data.push(buffer[idx]);
                    idx += 1;
                }
            }

            Ok(out_data)
        })
    }

    #[derive(FromArgs)]
    struct B2aQpArgs {
        #[pyarg(any)]
        data: ArgBytesLike,
        #[pyarg(named, default = false)]
        quotetabs: bool,
        #[pyarg(named, default = true)]
        istext: bool,
        #[pyarg(named, default = false)]
        header: bool,
    }

    #[pyfunction]
    fn b2a_qp(args: B2aQpArgs) -> PyResult<Vec<u8>> {
        let s = args.data;
        let quotetabs = args.quotetabs;
        let istext = args.istext;
        let header = args.header;
        s.with_ref(|buf| {
            let buflen = buf.len();
            let mut line_len = 0;
            let mut out_data_len = 0;
            let mut crlf = false;
            let mut ch;

            let mut in_idx;
            let mut out_idx;

            in_idx = 0;
            while in_idx < buflen {
                if buf[in_idx] == b'\n' {
                    break;
                }
                in_idx += 1;
            }
            if buflen > 0 && in_idx < buflen && buf[in_idx - 1] == b'\r' {
                crlf = true;
            }

            in_idx = 0;
            while in_idx < buflen {
                let mut delta = 0;
                if (buf[in_idx] > 126)
                    || (buf[in_idx] == b'=')
                    || (header && buf[in_idx] == b'_')
                    || (buf[in_idx] == b'.'
                        && line_len == 0
                        && (in_idx + 1 == buflen
                            || buf[in_idx + 1] == b'\n'
                            || buf[in_idx + 1] == b'\r'
                            || buf[in_idx + 1] == 0))
                    || (!istext && ((buf[in_idx] == b'\r') || (buf[in_idx] == b'\n')))
                    || ((buf[in_idx] == b'\t' || buf[in_idx] == b' ') && (in_idx + 1 == buflen))
                    || ((buf[in_idx] < 33)
                        && (buf[in_idx] != b'\r')
                        && (buf[in_idx] != b'\n')
                        && (quotetabs || ((buf[in_idx] != b'\t') && (buf[in_idx] != b' '))))
                {
                    if (line_len + 3) >= MAXLINESIZE {
                        line_len = 0;
                        delta += if crlf { 3 } else { 2 };
                    }
                    line_len += 3;
                    delta += 3;
                    in_idx += 1;
                } else if istext
                    && ((buf[in_idx] == b'\n')
                        || ((in_idx + 1 < buflen)
                            && (buf[in_idx] == b'\r')
                            && (buf[in_idx + 1] == b'\n')))
                {
                    line_len = 0;
                    // Protect against whitespace on end of line
                    if (in_idx != 0) && ((buf[in_idx - 1] == b' ') || (buf[in_idx - 1] == b'\t')) {
                        delta += 2;
                    }
                    delta += if crlf { 2 } else { 1 };
                    in_idx += if buf[in_idx] == b'\r' { 2 } else { 1 };
                } else {
                    if (in_idx + 1 != buflen)
                        && (buf[in_idx + 1] != b'\n')
                        && (line_len + 1) >= MAXLINESIZE
                    {
                        line_len = 0;
                        delta += if crlf { 3 } else { 2 };
                    }
                    line_len += 1;
                    delta += 1;
                    in_idx += 1;
                }
                out_data_len += delta;
            }

            let mut out_data = Vec::with_capacity(out_data_len);
            in_idx = 0;
            out_idx = 0;
            line_len = 0;

            while in_idx < buflen {
                if (buf[in_idx] > 126)
                    || (buf[in_idx] == b'=')
                    || (header && buf[in_idx] == b'_')
                    || ((buf[in_idx] == b'.')
                        && (line_len == 0)
                        && (in_idx + 1 == buflen
                            || buf[in_idx + 1] == b'\n'
                            || buf[in_idx + 1] == b'\r'
                            || buf[in_idx + 1] == 0))
                    || (!istext && ((buf[in_idx] == b'\r') || (buf[in_idx] == b'\n')))
                    || ((buf[in_idx] == b'\t' || buf[in_idx] == b' ') && (in_idx + 1 == buflen))
                    || ((buf[in_idx] < 33)
                        && (buf[in_idx] != b'\r')
                        && (buf[in_idx] != b'\n')
                        && (quotetabs || ((buf[in_idx] != b'\t') && (buf[in_idx] != b' '))))
                {
                    if (line_len + 3) >= MAXLINESIZE {
                        // MAXLINESIZE = 76
                        out_data.push(b'=');
                        out_idx += 1;
                        if crlf {
                            out_data.push(b'\r');
                            out_idx += 1;
                        }
                        out_data.push(b'\n');
                        out_idx += 1;
                        line_len = 0;
                    }
                    out_data.push(b'=');
                    out_idx += 1;

                    ch = hex_nibble(buf[in_idx] >> 4);
                    if (b'a'..=b'f').contains(&ch) {
                        ch -= b' ';
                    }
                    out_data.push(ch);
                    ch = hex_nibble(buf[in_idx] & 0xf);
                    if (b'a'..=b'f').contains(&ch) {
                        ch -= b' ';
                    }
                    out_data.push(ch);

                    out_idx += 2;
                    in_idx += 1;
                    line_len += 3;
                } else if istext
                    && ((buf[in_idx] == b'\n')
                        || ((in_idx + 1 < buflen)
                            && (buf[in_idx] == b'\r')
                            && (buf[in_idx + 1] == b'\n')))
                {
                    line_len = 0;
                    if (out_idx != 0)
                        && ((out_data[out_idx - 1] == b' ') || (out_data[out_idx - 1] == b'\t'))
                    {
                        ch = hex_nibble(out_data[out_idx - 1] >> 4);
                        if (b'a'..=b'f').contains(&ch) {
                            ch -= b' ';
                        }
                        out_data.push(ch);
                        ch = hex_nibble(out_data[out_idx - 1] & 0xf);
                        if (b'a'..=b'f').contains(&ch) {
                            ch -= b' ';
                        }
                        out_data.push(ch);
                        out_data[out_idx - 1] = b'=';
                        out_idx += 2;
                    }

                    if crlf {
                        out_data.push(b'\r');
                        out_idx += 1;
                    }
                    out_data.push(b'\n');
                    out_idx += 1;
                    in_idx += if buf[in_idx] == b'\r' { 2 } else { 1 };
                } else {
                    if (in_idx + 1 != buflen) && (buf[in_idx + 1] != b'\n') && (line_len + 1) >= 76
                    {
                        // MAXLINESIZE = 76
                        out_data.push(b'=');
                        out_idx += 1;
                        if crlf {
                            out_data.push(b'\r');
                            out_idx += 1;
                        }
                        out_data.push(b'\n');
                        out_idx += 1;
                        line_len = 0;
                    }
                    line_len += 1;
                    if header && buf[in_idx] == b' ' {
                        out_data.push(b'_');
                        out_idx += 1;
                        in_idx += 1;
                    } else {
                        out_data.push(buf[in_idx]);
                        out_idx += 1;
                        in_idx += 1;
                    }
                }
            }
            Ok(out_data)
        })
    }

    #[pyfunction]
    fn rlecode_hqx(s: ArgAsciiBuffer) -> PyResult<Vec<u8>> {
        const RUN_CHAR: u8 = 0x90; // b'\x90'
        s.with_ref(|buffer| {
            let len = buffer.len();
            let mut out_data = Vec::<u8>::with_capacity((len * 2) + 2);

            let mut idx = 0;
            while idx < len {
                let ch = buffer[idx];

                if ch == RUN_CHAR {
                    out_data.push(RUN_CHAR);
                    out_data.push(0);
                    return Ok(out_data);
                } else {
                    let mut in_end = idx + 1;
                    while in_end < len && buffer[in_end] == ch && in_end < idx + 255 {
                        in_end += 1;
                    }
                    if in_end - idx > 3 {
                        out_data.push(ch);
                        out_data.push(RUN_CHAR);
                        out_data.push(((in_end - idx) % 256) as u8);
                        idx = in_end - 1;
                    } else {
                        out_data.push(ch);
                    }
                }
                idx += 1;
            }
            Ok(out_data)
        })
    }

    #[pyfunction]
    fn rledecode_hqx(s: ArgAsciiBuffer) -> PyResult<Vec<u8>> {
        const RUN_CHAR: u8 = 0x90; //b'\x90'
        s.with_ref(|buffer| {
            let len = buffer.len();
            let mut out_data = Vec::<u8>::with_capacity(len);
            let mut idx = 0;

            out_data.push(buffer[idx]);
            idx += 1;

            while idx < len {
                if buffer[idx] == RUN_CHAR {
                    if buffer[idx + 1] == 0 {
                        out_data.push(RUN_CHAR);
                    } else {
                        let ch = buffer[idx - 1];
                        let range = buffer[idx + 1];
                        idx += 1;
                        for _ in 1..range {
                            out_data.push(ch);
                        }
                    }
                } else {
                    out_data.push(buffer[idx]);
                }
                idx += 1;
            }
            Ok(out_data)
        })
    }

    #[pyfunction]
    fn a2b_uu(s: ArgAsciiBuffer, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        s.with_ref(|b| {
            // First byte: binary data length (in bytes)
            let length = if b.is_empty() {
                ((-0x20i32) & 0x3fi32) as usize
            } else {
                ((b[0] - b' ') & 0x3f) as usize
            };

            // Allocate the buffer
            let mut res = Vec::<u8>::with_capacity(length);
            let trailing_garbage_error =
                || Err(super::new_binascii_error("Trailing garbage".to_owned(), vm));

            for chunk in b.get(1..).unwrap_or_default().chunks(4) {
                let (char_a, char_b, char_c, char_d) = {
                    let mut chunk = chunk
                        .iter()
                        .map(|x| uu_a2b_read(x, vm))
                        .collect::<Result<Vec<_>, _>>()?;
                    while chunk.len() < 4 {
                        chunk.push(0);
                    }
                    (chunk[0], chunk[1], chunk[2], chunk[3])
                };

                if res.len() < length {
                    res.push((char_a << 2) | (char_b >> 4));
                } else if char_a != 0 || char_b != 0 {
                    return trailing_garbage_error();
                }

                if res.len() < length {
                    res.push(((char_b & 0xf) << 4) | (char_c >> 2));
                } else if char_c != 0 {
                    return trailing_garbage_error();
                }

                if res.len() < length {
                    res.push(((char_c & 0x3) << 6) | char_d);
                } else if char_d != 0 {
                    return trailing_garbage_error();
                }
            }

            let remaining_length = length - res.len();
            if remaining_length > 0 {
                res.extend(vec![0; remaining_length]);
            }
            Ok(res)
        })
    }

    #[derive(FromArgs)]
    struct BacktickArg {
        #[pyarg(named, default = false)]
        backtick: bool,
    }

    #[pyfunction]
    fn b2a_uu(
        data: ArgBytesLike,
        BacktickArg { backtick }: BacktickArg,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<u8>> {
        #[inline]
        const fn uu_b2a(num: u8, backtick: bool) -> u8 {
            if backtick && num == 0 {
                0x60
            } else {
                b' ' + num
            }
        }

        data.with_ref(|b| {
            let length = b.len();
            if length > 45 {
                return Err(super::new_binascii_error(
                    "At most 45 bytes at once".to_owned(),
                    vm,
                ));
            }
            let mut res = Vec::<u8>::with_capacity(2 + length.div_ceil(3) * 4);
            res.push(uu_b2a(length as u8, backtick));

            for chunk in b.chunks(3) {
                let char_a = *chunk.first().unwrap();
                let char_b = *chunk.get(1).unwrap_or(&0);
                let char_c = *chunk.get(2).unwrap_or(&0);

                res.push(uu_b2a(char_a >> 2, backtick));
                res.push(uu_b2a(((char_a & 0x3) << 4) | (char_b >> 4), backtick));
                res.push(uu_b2a(((char_b & 0xf) << 2) | (char_c >> 6), backtick));
                res.push(uu_b2a(char_c & 0x3f, backtick));
            }

            res.push(0xau8);
            Ok(res)
        })
    }
}

struct Base64DecodeError(base64::DecodeError);

fn new_binascii_error(msg: String, vm: &VirtualMachine) -> PyBaseExceptionRef {
    vm.new_exception_msg(decl::error_type(vm), msg)
}

impl ToPyException for Base64DecodeError {
    fn to_pyexception(&self, vm: &VirtualMachine) -> PyBaseExceptionRef {
        use base64::DecodeError::*;
        let message = match &self.0 {
            InvalidByte(0, PAD) => "Leading padding not allowed".to_owned(),
            InvalidByte(_, PAD) => "Discontinuous padding not allowed".to_owned(),
            InvalidByte(_, _) => "Only base64 data is allowed".to_owned(),
            InvalidLastSymbol(_, PAD) => "Excess data after padding".to_owned(),
            InvalidLastSymbol(length, _) => {
                format!(
                    "Invalid base64-encoded string: number of data characters {length} cannot be 1 more than a multiple of 4"
                )
            }
            // TODO: clean up errors
            InvalidLength(_) => "Incorrect padding".to_owned(),
            InvalidPadding => "Incorrect padding".to_owned(),
        };
        new_binascii_error(format!("error decoding base64: {message}"), vm)
    }
}
