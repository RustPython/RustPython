pub(crate) use decl::make_module;

#[pymodule(name = "binascii")]
mod decl {
    use crate::byteslike::PyBytesLike;
    use crate::function::OptionalArg;
    use crate::obj::objbytearray::{PyByteArray, PyByteArrayRef};
    use crate::obj::objbytes::{PyBytes, PyBytesRef};
    use crate::obj::objstr::{PyString, PyStringRef};
    use crate::pyobject::{BorrowValue, PyObjectRef, PyResult, TryFromObject, TypeProtocol};
    use crate::vm::VirtualMachine;
    use crc::{crc32, Hasher32};
    use itertools::Itertools;

    enum SerializedData {
        Bytes(PyBytesRef),
        Buffer(PyByteArrayRef),
        Ascii(PyStringRef),
    }

    impl TryFromObject for SerializedData {
        fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
            match_class!(match obj {
                b @ PyBytes => Ok(SerializedData::Bytes(b)),
                b @ PyByteArray => Ok(SerializedData::Buffer(b)),
                a @ PyString => {
                    if a.borrow_value().is_ascii() {
                        Ok(SerializedData::Ascii(a))
                    } else {
                        Err(vm.new_value_error(
                            "string argument should contain only ASCII characters".to_owned(),
                        ))
                    }
                }
                obj => Err(vm.new_type_error(format!(
                    "argument should be bytes, buffer or ASCII string, not '{}'",
                    obj.class().name,
                ))),
            })
        }
    }

    impl SerializedData {
        #[inline]
        pub fn with_ref<R>(&self, f: impl FnOnce(&[u8]) -> R) -> R {
            match self {
                SerializedData::Bytes(b) => f(b.borrow_value()),
                SerializedData::Buffer(b) => f(&b.borrow_value().elements),
                SerializedData::Ascii(a) => f(a.borrow_value().as_bytes()),
            }
        }
    }

    fn hex_nibble(n: u8) -> u8 {
        match n {
            0..=9 => b'0' + n,
            10..=15 => b'a' + n,
            _ => unreachable!(),
        }
    }

    #[pyfunction(name = "b2a_hex")]
    #[pyfunction]
    fn hexlify(data: PyBytesLike) -> Vec<u8> {
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
        data.with_ref(|bytes| digest.write(&bytes));

        Ok(vm.ctx.new_int(digest.sum32()))
    }

    #[derive(FromArgs)]
    struct NewlineArg {
        #[pyarg(keyword_only, default = "true")]
        newline: bool,
    }

    /// trim a newline from the end of the bytestring, if it exists
    fn trim_newline(b: &[u8]) -> &[u8] {
        if b.ends_with(b"\n") {
            &b[..b.len() - 1]
        } else {
            b
        }
    }

    #[pyfunction]
    fn a2b_base64(s: SerializedData, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        s.with_ref(|b| base64::decode(trim_newline(b)))
            .map_err(|err| vm.new_value_error(format!("error decoding base64: {}", err)))
    }

    #[pyfunction]
    fn b2a_base64(data: PyBytesLike, NewlineArg { newline }: NewlineArg) -> Vec<u8> {
        let mut encoded = data.with_ref(base64::encode).into_bytes();
        if newline {
            encoded.push(b'\n');
        }
        encoded
    }
}
