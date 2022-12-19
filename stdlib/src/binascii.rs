pub(crate) use decl::make_module;

pub(super) use decl::crc32;

pub fn decode<T: AsRef<[u8]>>(input: T) -> Result<Vec<u8>, base64::DecodeError> {
    base64::decode_config(input, base64::STANDARD.decode_allow_trailing_bits(true))
}

#[pymodule(name = "binascii")]
mod decl {
    use super::decode;
    use crate::vm::{
        builtins::{PyBaseExceptionRef, PyIntRef, PyTypeRef},
        function::{ArgAsciiBuffer, ArgBytesLike, OptionalArg},
        PyResult, VirtualMachine,
    };
    use itertools::Itertools;

    const MAXLINESIZE: usize = 76;

    #[pyattr(name = "Error", once)]
    fn error_type(vm: &VirtualMachine) -> PyTypeRef {
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
    fn hexlify(data: ArgBytesLike) -> Vec<u8> {
        data.with_ref(|bytes| {
            let mut hex = Vec::<u8>::with_capacity(bytes.len() * 2);
            for b in bytes {
                hex.push(hex_nibble(b >> 4));
                hex.push(hex_nibble(b & 0xf));
            }
            hex
        })
    }

    fn unhex_nibble(c: u8) -> Option<u8> {
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
                return Err(new_binascii_error("Odd-length string".to_owned(), vm));
            }

            let mut unhex = Vec::<u8>::with_capacity(hex_bytes.len() / 2);
            for (n1, n2) in hex_bytes.iter().tuples() {
                if let (Some(n1), Some(n2)) = (unhex_nibble(*n1), unhex_nibble(*n2)) {
                    unhex.push(n1 << 4 | n2);
                } else {
                    return Err(new_binascii_error(
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
        #[pyarg(named, default = "true")]
        newline: bool,
    }

    fn new_binascii_error(msg: String, vm: &VirtualMachine) -> PyBaseExceptionRef {
        vm.new_exception_msg(error_type(vm), msg)
    }

    #[pyfunction]
    fn a2b_base64(s: ArgAsciiBuffer, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        #[rustfmt::skip]
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

        s.with_ref(|b| {
            let decoded = if b.len() % 4 == 0 {
                decode(b)
            } else {
                Err(base64::DecodeError::InvalidLength)
            };
            decoded.or_else(|_| {
                let buf: Vec<_> = b
                    .iter()
                    .copied()
                    .filter(|&c| BASE64_TABLE[c as usize] != -1)
                    .collect();
                if buf.len() % 4 != 0 {
                    return Err(base64::DecodeError::InvalidLength);
                }
                decode(&buf)
            })
        })
        .map_err(|err| new_binascii_error(format!("error decoding base64: {err}"), vm))
    }

    #[pyfunction]
    fn b2a_base64(data: ArgBytesLike, NewlineArg { newline }: NewlineArg) -> Vec<u8> {
        // https://stackoverflow.com/questions/63916821
        let mut encoded = data.with_ref(|b| base64::encode(b)).into_bytes();
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
            return Err(vm.new_value_error("Illegal char".to_string()));
        }
        Ok((*c - b' ') & 0x3f)
    }

    #[derive(FromArgs)]
    struct A2bQpArgs {
        #[pyarg(any)]
        data: ArgAsciiBuffer,
        #[pyarg(named, default = "false")]
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
                        // roken case from broken python qp
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
                        // hexval
                        if let (Some(ch1), Some(ch2)) =
                            (unhex_nibble(buffer[idx]), unhex_nibble(buffer[idx + 1]))
                        {
                            out_data.push(ch1 << 4 | ch2);
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
        data: ArgAsciiBuffer,
        #[pyarg(named, default = "false")]
        quotetabs: bool,
        #[pyarg(named, default = "true")]
        istext: bool,
        #[pyarg(named, default = "false")]
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
            let mut linelen = 0;
            let mut odatalen = 0;
            let mut crlf = false;
            let mut ch;

            let mut inidx;
            let mut outidx;

            inidx = 0;
            while inidx < buflen {
                if buf[inidx] == b'\n' {
                    break;
                }
                inidx += 1;
            }
            if buflen > 0 && inidx < buflen && buf[inidx - 1] == b'\r' {
                crlf = true;
            }

            inidx = 0;
            while inidx < buflen {
                let mut delta = 0;
                if (buf[inidx] > 126)
                    || (buf[inidx] == b'=')
                    || (header && buf[inidx] == b'_')
                    || (buf[inidx] == b'.'
                        && linelen == 0
                        && (inidx + 1 == buflen
                            || buf[inidx + 1] == b'\n'
                            || buf[inidx + 1] == b'\r'
                            || buf[inidx + 1] == 0))
                    || (!istext && ((buf[inidx] == b'\r') || (buf[inidx] == b'\n')))
                    || ((buf[inidx] == b'\t' || buf[inidx] == b' ') && (inidx + 1 == buflen))
                    || ((buf[inidx] < 33)
                        && (buf[inidx] != b'\r')
                        && (buf[inidx] != b'\n')
                        && (quotetabs || ((buf[inidx] != b'\t') && (buf[inidx] != b' '))))
                {
                    if (linelen + 3) >= MAXLINESIZE {
                        linelen = 0;
                        delta += if crlf { 3 } else { 2 };
                    }
                    linelen += 3;
                    delta += 3;
                    inidx += 1;
                } else if istext
                    && ((buf[inidx] == b'\n')
                        || ((inidx + 1 < buflen)
                            && (buf[inidx] == b'\r')
                            && (buf[inidx + 1] == b'\n')))
                {
                    linelen = 0;
                    // Protect against whitespace on end of line
                    if (inidx != 0) && ((buf[inidx - 1] == b' ') || (buf[inidx - 1] == b'\t')) {
                        delta += 2;
                    }
                    delta += if crlf { 2 } else { 1 };
                    inidx += if buf[inidx] == b'\r' { 2 } else { 1 };
                } else {
                    if (inidx + 1 != buflen)
                        && (buf[inidx + 1] != b'\n')
                        && (linelen + 1) >= MAXLINESIZE
                    {
                        linelen = 0;
                        delta += if crlf { 3 } else { 2 };
                    }
                    linelen += 1;
                    delta += 1;
                    inidx += 1;
                }
                odatalen += delta;
            }

            let mut out_data = Vec::with_capacity(odatalen);
            inidx = 0;
            outidx = 0;
            linelen = 0;

            while inidx < buflen {
                if (buf[inidx] > 126)
                    || (buf[inidx] == b'=')
                    || (header && buf[inidx] == b'_')
                    || ((buf[inidx] == b'.')
                        && (linelen == 0)
                        && (inidx + 1 == buflen
                            || buf[inidx + 1] == b'\n'
                            || buf[inidx + 1] == b'\r'
                            || buf[inidx + 1] == 0))
                    || (!istext && ((buf[inidx] == b'\r') || (buf[inidx] == b'\n')))
                    || ((buf[inidx] == b'\t' || buf[inidx] == b' ') && (inidx + 1 == buflen))
                    || ((buf[inidx] < 33)
                        && (buf[inidx] != b'\r')
                        && (buf[inidx] != b'\n')
                        && (quotetabs || ((buf[inidx] != b'\t') && (buf[inidx] != b' '))))
                {
                    if (linelen + 3) >= MAXLINESIZE {
                        // MAXLINESIZE = 76
                        out_data.push(b'=');
                        outidx += 1;
                        if crlf {
                            out_data.push(b'\r');
                            outidx += 1;
                        }
                        out_data.push(b'\n');
                        outidx += 1;
                        linelen = 0;
                    }
                    out_data.push(b'=');
                    outidx += 1;

                    ch = hex_nibble(buf[inidx] >> 4);
                    if (b'a'..=b'f').contains(&ch) {
                        ch -= b' ';
                    }
                    out_data.push(ch);
                    ch = hex_nibble(buf[inidx] & 0xf);
                    if (b'a'..=b'f').contains(&ch) {
                        ch -= b' ';
                    }
                    out_data.push(ch);

                    outidx += 2;
                    inidx += 1;
                    linelen += 3;
                } else if istext
                    && ((buf[inidx] == b'\n')
                        || ((inidx + 1 < buflen)
                            && (buf[inidx] == b'\r')
                            && (buf[inidx + 1] == b'\n')))
                {
                    linelen = 0;
                    if (outidx != 0)
                        && ((out_data[outidx - 1] == b' ') || (out_data[outidx - 1] == b'\t'))
                    {
                        ch = hex_nibble(out_data[outidx - 1] >> 4);
                        if (b'a'..=b'f').contains(&ch) {
                            ch -= b' ';
                        }
                        out_data.push(ch);
                        ch = hex_nibble(out_data[outidx - 1] & 0xf);
                        if (b'a'..=b'f').contains(&ch) {
                            ch -= b' ';
                        }
                        out_data.push(ch);
                        out_data[outidx - 1] = b'=';
                        outidx += 2;
                    }

                    if crlf {
                        out_data.push(b'\r');
                        outidx += 1;
                    }
                    out_data.push(b'\n');
                    outidx += 1;
                    inidx += if buf[inidx] == b'\r' { 2 } else { 1 };
                } else {
                    if (inidx + 1 != buflen) && (buf[inidx + 1] != b'\n') && (linelen + 1) >= 76 {
                        // MAXLINESIZE = 76
                        out_data.push(b'=');
                        outidx += 1;
                        if crlf {
                            out_data.push(b'\r');
                            outidx += 1;
                        }
                        out_data.push(b'\n');
                        outidx += 1;
                        linelen = 0;
                    }
                    linelen += 1;
                    if header && buf[inidx] == b' ' {
                        out_data.push(b'_');
                        outidx += 1;
                        inidx += 1;
                    } else {
                        out_data.push(buf[inidx]);
                        outidx += 1;
                        inidx += 1;
                    }
                }
            }
            Ok(out_data)
        })
    }

    #[pyfunction]
    fn rlecode_hqx(s: ArgAsciiBuffer) -> PyResult<Vec<u8>> {
        const RUNCHAR: u8 = 0x90; // b'\x90'
        s.with_ref(|buffer| {
            let len = buffer.len();
            let mut out_data = Vec::<u8>::with_capacity((len * 2) + 2);

            let mut idx = 0;
            while idx < len {
                let ch = buffer[idx];

                if ch == RUNCHAR {
                    out_data.push(RUNCHAR);
                    out_data.push(0);
                    return Ok(out_data);
                } else {
                    let mut inend = idx + 1;
                    while inend < len && buffer[inend] == ch && inend < idx + 255 {
                        inend += 1;
                    }
                    if inend - idx > 3 {
                        out_data.push(ch);
                        out_data.push(RUNCHAR);
                        out_data.push(((inend - idx) % 256) as u8);
                        idx = inend - 1;
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
        const RUNCHAR: u8 = 0x90; //b'\x90'
        s.with_ref(|buffer| {
            let len = buffer.len();
            let mut out_data = Vec::<u8>::with_capacity(len);
            let mut idx = 0;

            out_data.push(buffer[idx]);
            idx += 1;

            while idx < len {
                if buffer[idx] == RUNCHAR {
                    if buffer[idx + 1] == 0 {
                        out_data.push(RUNCHAR);
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
            let trailing_garbage_error = || Err(vm.new_value_error("Trailing garbage".to_string()));

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
                    res.push(char_a << 2 | char_b >> 4);
                } else if char_a != 0 || char_b != 0 {
                    return trailing_garbage_error();
                }

                if res.len() < length {
                    res.push((char_b & 0xf) | char_c >> 2);
                } else if char_c != 0 {
                    return trailing_garbage_error();
                }

                if res.len() < length {
                    res.push((char_c & 0x3) << 6 | char_d);
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
        #[pyarg(named, default = "true")]
        backtick: bool,
    }

    #[pyfunction]
    fn b2a_uu(
        data: ArgBytesLike,
        BacktickArg { backtick }: BacktickArg,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<u8>> {
        #[inline]
        fn uu_b2a(num: u8, backtick: bool) -> u8 {
            if backtick && num != 0 {
                0x60
            } else {
                b' ' + num
            }
        }

        data.with_ref(|b| {
            let length = b.len();
            if length > 45 {
                return Err(vm.new_value_error("At most 45 bytes at once".to_string()));
            }
            let mut res = Vec::<u8>::with_capacity(2 + ((length + 2) / 3) * 4);
            res.push(uu_b2a(length as u8, backtick));

            for chunk in b.chunks(3) {
                let char_a = *chunk.first().unwrap();
                let char_b = *chunk.get(1).unwrap_or(&0);
                let char_c = *chunk.get(2).unwrap_or(&0);

                res.push(uu_b2a(char_a >> 2, backtick));
                res.push(uu_b2a((char_a & 0x3) << 4 | char_b >> 4, backtick));
                res.push(uu_b2a((char_b & 0xf) << 2 | char_c >> 6, backtick));
                res.push(uu_b2a(char_c & 0x3f, backtick));
            }

            res.push(0xau8);
            Ok(res)
        })
    }
}
