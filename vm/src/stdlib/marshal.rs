pub(crate) use decl::make_module;

#[pymodule(name = "marshal")]
mod decl {
    use crate::{
        builtins::{
            PyBaseExceptionRef, PyByteArray, PyBytes, PyCode, PyDict, PyFloat, PyFrozenSet, PyInt,
            PyList, PySet, PyStr, PyTuple,
        },
        bytecode,
        convert::ToPyObject,
        function::ArgBytesLike,
        object::AsObject,
        protocol::PyBuffer,
        PyObjectRef, PyResult, TryFromObject, VirtualMachine,
    };
    use num_bigint::{BigInt, Sign};
    use num_traits::Zero;

    #[repr(u8)]
    enum Type {
        // Null = b'0',
        None = b'N',
        False = b'F',
        True = b'T',
        // StopIter = b'S',
        Ellipsis = b'.',
        Int = b'i',
        Float = b'g',
        // Complex = b'y',
        // Long = b'l',  // i32
        Bytes = b's', // = TYPE_STRING
        // Interned = b't',
        // Ref = b'r',
        Tuple = b'(',
        List = b'[',
        Dict = b'{',
        Code = b'c',
        Unicode = b'u',
        // Unknown = b'?',
        Set = b'<',
        FrozenSet = b'>',
        Ascii = b'a',
        // AsciiInterned = b'A',
        // SmallTuple = b')',
        // ShortAscii = b'z',
        // ShortAsciiInterned = b'Z',
    }
    // const FLAG_REF: u8 = b'\x80';

    impl TryFrom<u8> for Type {
        type Error = u8;
        fn try_from(value: u8) -> Result<Self, u8> {
            use Type::*;
            Ok(match value {
                // b'0' => Null,
                b'N' => None,
                b'F' => False,
                b'T' => True,
                // b'S' => StopIter,
                b'.' => Ellipsis,
                b'i' => Int,
                b'g' => Float,
                // b'y' => Complex,
                // b'l' => Long,
                b's' => Bytes,
                // b't' => Interned,
                // b'r' => Ref,
                b'(' => Tuple,
                b'[' => List,
                b'{' => Dict,
                b'c' => Code,
                b'u' => Unicode,
                // b'?' => Unknown,
                b'<' => Set,
                b'>' => FrozenSet,
                b'a' => Ascii,
                // b'A' => AsciiInterned,
                // b')' => SmallTuple,
                // b'z' => ShortAscii,
                // b'Z' => ShortAsciiInterned,
                c => return Err(c),
            })
        }
    }

    fn too_short_error(vm: &VirtualMachine) -> PyBaseExceptionRef {
        vm.new_exception_msg(
            vm.ctx.exceptions.eof_error.to_owned(),
            "marshal data too short".to_owned(),
        )
    }

    /// Dumps a sequence of objects into binary vector.
    fn dump_seq(
        buf: &mut Vec<u8>,
        iter: std::slice::Iter<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        write_size(buf, iter.len(), vm)?;
        // For each element, dump into binary, then add its length and value.
        for element in iter {
            dump_obj(buf, element.clone(), vm)?;
        }
        Ok(())
    }

    /// Dumping helper function to turn a value into bytes.
    fn dump_obj(buf: &mut Vec<u8>, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        if vm.is_none(&value) {
            buf.push(Type::None as u8);
        } else if value.is(&vm.ctx.ellipsis) {
            buf.push(Type::Ellipsis as u8);
        } else {
            match_class!(match value {
                pyint @ PyInt => {
                    if pyint.class().is(vm.ctx.types.bool_type) {
                        let typ = if pyint.as_bigint().is_zero() {
                            Type::False
                        } else {
                            Type::True
                        };
                        buf.push(typ as u8);
                    } else {
                        buf.push(Type::Int as u8);
                        let (sign, int_bytes) = pyint.as_bigint().to_bytes_le();
                        let mut len = int_bytes.len() as i32;
                        if sign == Sign::Minus {
                            len = -len;
                        }
                        buf.extend(len.to_le_bytes());
                        buf.extend(int_bytes);
                    }
                }
                pyfloat @ PyFloat => {
                    buf.push(Type::Float as u8);
                    buf.extend(pyfloat.to_f64().to_le_bytes());
                }
                pystr @ PyStr => {
                    buf.push(if pystr.is_ascii() {
                        Type::Ascii
                    } else {
                        Type::Unicode
                    } as u8);
                    write_size(buf, pystr.as_str().len(), vm)?;
                    buf.extend(pystr.as_str().as_bytes());
                }
                pylist @ PyList => {
                    buf.push(Type::List as u8);
                    let pylist_items = pylist.borrow_vec();
                    dump_seq(buf, pylist_items.iter(), vm)?;
                }
                pyset @ PySet => {
                    buf.push(Type::Set as u8);
                    let elements = pyset.elements();
                    dump_seq(buf, elements.iter(), vm)?;
                }
                pyfrozen @ PyFrozenSet => {
                    buf.push(Type::FrozenSet as u8);
                    let elements = pyfrozen.elements();
                    dump_seq(buf, elements.iter(), vm)?;
                }
                pytuple @ PyTuple => {
                    buf.push(Type::Tuple as u8);
                    dump_seq(buf, pytuple.iter(), vm)?;
                }
                pydict @ PyDict => {
                    buf.push(Type::Dict as u8);
                    write_size(buf, pydict.len(), vm)?;
                    for (key, value) in pydict {
                        dump_obj(buf, key, vm)?;
                        dump_obj(buf, value, vm)?;
                    }
                }
                bytes @ PyBytes => {
                    buf.push(Type::Bytes as u8);
                    let data = bytes.as_bytes();
                    write_size(buf, data.len(), vm)?;
                    buf.extend(&*data);
                }
                bytes @ PyByteArray => {
                    buf.push(Type::Bytes as u8);
                    let data = bytes.borrow_buf();
                    write_size(buf, data.len(), vm)?;
                    buf.extend(&*data);
                }
                co @ PyCode => {
                    buf.push(Type::Code as u8);
                    let bytes = co.code.map_clone_bag(&bytecode::BasicBag).to_bytes();
                    write_size(buf, bytes.len(), vm)?;
                    buf.extend(bytes);
                }
                _ => {
                    return Err(vm.new_not_implemented_error(
                        "TODO: not implemented yet or marshal unsupported type".to_owned(),
                    ));
                }
            })
        }
        Ok(())
    }

    #[pyfunction]
    fn dumps(value: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyBytes> {
        let mut buf = Vec::new();
        dump_obj(&mut buf, value, vm)?;
        Ok(PyBytes::from(buf))
    }

    #[pyfunction]
    fn dump(value: PyObjectRef, f: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let dumped = dumps(value, vm)?;
        vm.call_method(&f, "write", (dumped,))?;
        Ok(())
    }

    /// Safely convert usize to 4 le bytes
    fn write_size(buf: &mut Vec<u8>, x: usize, vm: &VirtualMachine) -> PyResult<()> {
        // For marshalling we want to convert lengths to bytes. To save space
        // we limit the size to u32 to keep marshalling smaller.
        let n = u32::try_from(x).map_err(|_| {
            vm.new_value_error("Size exceeds 2^32 capacity for marshaling.".to_owned())
        })?;
        buf.extend(n.to_le_bytes());
        Ok(())
    }

    /// Read the next 4 bytes of a slice, read as u32, pass as usize.
    /// Returns the rest of buffer with the value.
    fn read_size<'a>(buf: &'a [u8], vm: &VirtualMachine) -> PyResult<(usize, &'a [u8])> {
        if buf.len() < 4 {
            return Err(too_short_error(vm));
        }
        let (u32_bytes, rest) = buf.split_at(4);
        let length = u32::from_le_bytes(u32_bytes.try_into().unwrap());
        Ok((length as usize, rest))
    }

    /// Reads a list (or tuple) from a buffer.
    fn load_seq<'b>(buf: &'b [u8], vm: &VirtualMachine) -> PyResult<(Vec<PyObjectRef>, &'b [u8])> {
        let (len, mut buf) = read_size(buf, vm)?;
        let mut elements: Vec<PyObjectRef> = Vec::new();
        for _ in 0..len {
            let (element, rest) = load_obj(buf, vm)?;
            buf = rest;
            elements.push(element);
        }
        Ok((elements, buf))
    }

    #[pyfunction]
    fn loads(pybuffer: PyBuffer, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let buf = pybuffer.as_contiguous().ok_or_else(|| {
            vm.new_buffer_error("Buffer provided to marshal.loads() is not contiguous".to_owned())
        })?;
        let (obj, _) = load_obj(&buf, vm)?;
        Ok(obj)
    }

    fn load_obj<'b>(buf: &'b [u8], vm: &VirtualMachine) -> PyResult<(PyObjectRef, &'b [u8])> {
        let (type_indicator, buf) = buf.split_first().ok_or_else(|| too_short_error(vm))?;
        let typ = Type::try_from(*type_indicator)
            .map_err(|_| vm.new_value_error("bad marshal data (unknown type code)".to_owned()))?;
        let (obj, buf) = match typ {
            Type::True => (true.to_pyobject(vm), buf),
            Type::False => (false.to_pyobject(vm), buf),
            Type::None => (vm.ctx.none(), buf),
            Type::Ellipsis => (vm.ctx.ellipsis(), buf),
            Type::Int => {
                if buf.len() < 4 {
                    return Err(too_short_error(vm));
                }
                let (len_bytes, buf) = buf.split_at(4);
                let len = i32::from_le_bytes(len_bytes.try_into().unwrap());
                let (sign, len) = if len < 0 {
                    (Sign::Minus, (-len) as usize)
                } else {
                    (Sign::Plus, len as usize)
                };
                if buf.len() < len {
                    return Err(too_short_error(vm));
                }
                let (bytes, buf) = buf.split_at(len);
                let int = BigInt::from_bytes_le(sign, bytes);
                (int.to_pyobject(vm), buf)
            }
            Type::Float => {
                if buf.len() < 8 {
                    return Err(too_short_error(vm));
                }
                let (bytes, buf) = buf.split_at(8);
                let number = f64::from_le_bytes(bytes.try_into().unwrap());
                (vm.ctx.new_float(number).into(), buf)
            }
            Type::Ascii => {
                let (len, buf) = read_size(buf, vm)?;
                if buf.len() < len {
                    return Err(too_short_error(vm));
                }
                let (bytes, buf) = buf.split_at(len);
                let s = String::from_utf8(bytes.to_vec())
                    .map_err(|_| vm.new_value_error("invalid utf8 data".to_owned()))?;
                (s.to_pyobject(vm), buf)
            }
            Type::Unicode => {
                let (len, buf) = read_size(buf, vm)?;
                if buf.len() < len {
                    return Err(too_short_error(vm));
                }
                let (bytes, buf) = buf.split_at(len);
                let s = String::from_utf8(bytes.to_vec())
                    .map_err(|_| vm.new_value_error("invalid utf8 data".to_owned()))?;
                (s.to_pyobject(vm), buf)
            }
            Type::List => {
                let (elements, buf) = load_seq(buf, vm)?;
                (vm.ctx.new_list(elements).into(), buf)
            }
            Type::Set => {
                let (elements, buf) = load_seq(buf, vm)?;
                let set = PySet::new_ref(&vm.ctx);
                for element in elements {
                    set.add(element, vm)?;
                }
                (set.to_pyobject(vm), buf)
            }
            Type::FrozenSet => {
                let (elements, buf) = load_seq(buf, vm)?;
                let set = PyFrozenSet::from_iter(vm, elements.into_iter())?;
                (set.to_pyobject(vm), buf)
            }
            Type::Tuple => {
                let (elements, buf) = load_seq(buf, vm)?;
                (vm.ctx.new_tuple(elements).into(), buf)
            }
            Type::Dict => {
                let (len, mut buf) = read_size(buf, vm)?;
                let dict = vm.ctx.new_dict();
                for _ in 0..len {
                    let (key, rest) = load_obj(buf, vm)?;
                    let (value, rest) = load_obj(rest, vm)?;
                    buf = rest;
                    dict.set_item(key.as_object(), value, vm)?;
                }
                (dict.into(), buf)
            }
            Type::Bytes => {
                // Following CPython, after marshaling, byte arrays are converted into bytes.
                let (len, buf) = read_size(buf, vm)?;
                if buf.len() < len {
                    return Err(too_short_error(vm));
                }
                let (bytes, buf) = buf.split_at(len);
                (vm.ctx.new_bytes(bytes.to_vec()).into(), buf)
            }
            Type::Code => {
                // If prefix is not identifiable, assume CodeObject, error out if it doesn't match.
                let (len, buf) = read_size(buf, vm)?;
                if buf.len() < len {
                    return Err(too_short_error(vm));
                }
                let (bytes, buf) = buf.split_at(len);
                let code = bytecode::CodeObject::from_bytes(bytes).map_err(|e| match e {
                    bytecode::CodeDeserializeError::Eof => vm.new_exception_msg(
                        vm.ctx.exceptions.eof_error.to_owned(),
                        "End of file while deserializing bytecode".to_owned(),
                    ),
                    _ => vm.new_value_error("Couldn't deserialize python bytecode".to_owned()),
                })?;
                (vm.ctx.new_code(code).into(), buf)
            }
        };
        Ok((obj, buf))
    }

    #[pyfunction]
    fn load(f: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let read_res = vm.call_method(&f, "read", ())?;
        let bytes = ArgBytesLike::try_from_object(vm, read_res)?;
        loads(PyBuffer::from(bytes), vm)
    }
}
