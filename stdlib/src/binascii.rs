pub(crate) use decl::make_module;

#[pymodule(name = "binascii")]
mod decl {
    use crate::vm::{
        builtins::{PyByteArray, PyBytes, PyStr, PyTypeRef},
        function::{ArgBytesLike, OptionalArg},
        match_class, PyObjectRef, PyRef, PyResult, TryFromObject, TypeProtocol, VirtualMachine,
    };
    use crc::{crc32, Hasher32};
    use itertools::Itertools;

    enum SerializedData {
        Bytes(PyRef<PyBytes>),
        Buffer(PyRef<PyByteArray>),
        Ascii(PyRef<PyStr>),
    }

    impl TryFromObject for SerializedData {
        fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
            match_class!(match obj {
                b @ PyBytes => Ok(SerializedData::Bytes(b)),
                b @ PyByteArray => Ok(SerializedData::Buffer(b)),
                a @ PyStr => {
                    if a.as_str().is_ascii() {
                        Ok(SerializedData::Ascii(a))
                    } else {
                        Err(vm.new_value_error(
                            "string argument should contain only ASCII characters".to_owned(),
                        ))
                    }
                }
                obj => Err(vm.new_type_error(format!(
                    "argument should be bytes, buffer or ASCII string, not '{}'",
                    obj.class().name(),
                ))),
            })
        }
    }

    impl SerializedData {
        #[inline]
        pub fn with_ref<R>(&self, f: impl FnOnce(&[u8]) -> R) -> R {
            match self {
                SerializedData::Bytes(b) => f(b.as_bytes()),
                SerializedData::Buffer(b) => f(&b.borrow_buf()),
                SerializedData::Ascii(a) => f(a.as_str().as_bytes()),
            }
        }
    }

    #[pyattr(name = "Error")]
    fn get_binascii_error(vm: &VirtualMachine) -> PyTypeRef {
        rustpython_common::static_cell! {
            static BINASCII_ERROR: PyTypeRef;
        }
        BINASCII_ERROR
            .get_or_init(|| {
                vm.ctx.new_class(
                    "binascii.Error",
                    &vm.ctx.exceptions.value_error,
                    Default::default(),
                )
            })
            .clone()
    }

    #[pyattr(name = "Incomplete")]
    fn get_binascii_incomplete(vm: &VirtualMachine) -> PyTypeRef {
        rustpython_common::static_cell! {
            static BINASCII_INCOMPLTE: PyTypeRef;
        }
        BINASCII_INCOMPLTE
            .get_or_init(|| {
                vm.ctx.new_class(
                    "binascii.Incomplete",
                    &vm.ctx.exceptions.exception_type,
                    Default::default(),
                )
            })
            .clone()
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
            for b in bytes.iter() {
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
    fn unhexlify(data: SerializedData, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
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
    fn crc32(data: SerializedData, value: OptionalArg<u32>, vm: &VirtualMachine) -> PyResult {
        let crc = value.unwrap_or(0);

        let mut digest = crc32::Digest::new_with_initial(crc32::IEEE, crc);
        data.with_ref(|bytes| digest.write(bytes));

        Ok(vm.ctx.new_int(digest.sum32()))
    }

    #[derive(FromArgs)]
    struct NewlineArg {
        #[pyarg(named, default = "true")]
        newline: bool,
    }

    #[pyfunction]
    fn a2b_base64(s: SerializedData, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        s.with_ref(|b| {
            let mut buf;
            let b = if memchr::memchr(b'\n', b).is_some() {
                buf = b.to_vec();
                buf.retain(|c| *c != b'\n');
                &buf
            } else {
                b
            };
            base64::decode(b)
        })
        .map_err(|err| vm.new_value_error(format!("error decoding base64: {}", err)))
    }

    #[pyfunction]
    fn b2a_base64(data: ArgBytesLike, NewlineArg { newline }: NewlineArg) -> Vec<u8> {
        #[allow(clippy::redundant_closure)] // https://stackoverflow.com/questions/63916821
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
    fn a2b_uu(s: SerializedData, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
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
