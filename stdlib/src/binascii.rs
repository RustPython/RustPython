pub(crate) use decl::make_module;

pub(super) use decl::crc32;

#[pymodule(name = "binascii")]
mod decl {
    use crate::vm::{
        builtins::{PyBaseExceptionRef, PyIntRef, PyTypeRef},
        function::{ArgAsciiBuffer, ArgBytesLike, OptionalArg},
        PyResult, VirtualMachine,
    };
    use itertools::Itertools;

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
                return Err(vm.new_value_error("Odd-length string".to_owned()));
            }

            let mut unhex = Vec::<u8>::with_capacity(hex_bytes.len() / 2);
            for (n1, n2) in hex_bytes.iter().tuples() {
                if let (Some(n1), Some(n2)) = (unhex_nibble(*n1), unhex_nibble(*n2)) {
                    unhex.push(n1 << 4 | n2);
                } else {
                    return Err(vm.new_value_error("Non-hexadecimal digit found".to_owned()));
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
        s.with_ref(|b| {
            let mut buf;
            let b = if memchr::memchr(b'\n', b).is_some() {
                buf = b.to_vec();
                buf.retain(|c| *c != b'\n');
                &buf
            } else {
                b
            };
            if b.len() % 4 != 0 {
                return Err(base64::DecodeError::InvalidLength);
            }
            base64::decode(b)
        })
        .map_err(|err| new_binascii_error(format!("error decoding base64: {}", err), vm))
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
        if !(0x20..=0x60).contains(c) {
            if [b'\r', b'\n'].contains(c) {
                return Ok(0);
            }
            return Err(vm.new_value_error("Illegal char".to_string()));
        }
        Ok((*c - 0x20) & 0x3f)
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

                if ch == runchar {
                    out_data.push(runchar);
                    out_data.push(0);
                    return Ok(out_data);
                } else {
                    let mut inend = idx + 1;
                    while inend < len && buffer[inend] == ch && inend < idx + 255 {
                        inend += 1;
                    }
                    if inend - idx > 3 {
                        out_data.push(ch);
                        out_data.push(runchar);
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
        let runchar = 0x90; //RUNCHAR = b"\x90"
        s.with_ref(|buffer| {
            let len = buffer.len();
            let mut out_data = Vec::<u8>::with_capacity(len);
            let mut idx = 0;

            if buffer[idx] == runchar {
                out_data.push(runchar);
            } else {
                out_data.push(buffer[idx]);
            }
            idx += 1;

            while idx < len {
                if buffer[idx] == runchar {
                    if buffer[idx + 1] == 0 {
                        out_data.push(runchar);
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
                ((b[0] - 0x20) & 0x3f) as usize
            };

            // Allocate the buffer
            let mut res = Vec::<u8>::with_capacity(length);
            let trailing_garbage_error = || Err(vm.new_value_error("Trailing garbage".to_string()));

            for chunk in b.get(1..).unwrap_or_default().chunks(4) {
                let char_a = chunk.get(0).map_or(Ok(0), |x| uu_a2b_read(x, vm))?;
                let char_b = chunk.get(1).map_or(Ok(0), |x| uu_a2b_read(x, vm))?;
                let char_c = chunk.get(2).map_or(Ok(0), |x| uu_a2b_read(x, vm))?;
                let char_d = chunk.get(3).map_or(Ok(0), |x| uu_a2b_read(x, vm))?;

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
                0x20 + num
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
                let char_a = *chunk.get(0).unwrap_or(&0);
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
