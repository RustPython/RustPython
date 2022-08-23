pub(crate) use decl::make_module;

#[pymodule(name = "marshal")]
mod decl {
    use crate::{
        builtins::{
            PyBaseExceptionRef, PyByteArray, PyBytes, PyCode, PyDict, PyFloat, PyFrozenSet, PyInt,
            PyList, PySet, PyStr, PyTuple,
        },
        bytecode,
        convert::{IntoPyException, ToPyObject, ToPyResult},
        function::{ArgBytesLike, OptionalArg},
        object::AsObject,
        protocol::PyBuffer,
        PyObjectRef, PyResult, TryFromObject, VirtualMachine,
    };
    use num_bigint::{BigInt, Sign};
    use num_traits::Zero;
    use std::str;

    #[derive(num_enum::TryFromPrimitive)]
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

    #[pyattr(name = "version")]
    const VERSION: u32 = 4;

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
                    buf.extend(data);
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
    fn dumps(
        value: PyObjectRef,
        _version: OptionalArg<i32>,
        vm: &VirtualMachine,
    ) -> PyResult<PyBytes> {
        let mut buf = Vec::new();
        dump_obj(&mut buf, value, vm)?;
        Ok(PyBytes::from(buf))
    }

    #[pyfunction]
    fn dump(
        value: PyObjectRef,
        f: PyObjectRef,
        version: OptionalArg<i32>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let dumped = dumps(value, version, vm)?;
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

    enum MarshalReadError {
        TooShort,
        Utf8,
        Ascii,
        UnknownType,
        Bytecode(bytecode::CodeDeserializeError),
        BufferDiscontiguous,
    }
    type MarshalReadResult<T> = Result<T, MarshalReadError>;
    macro_rules! impl_from {
        ($t:ty => $variant:ident$(($($x:tt)*))?) => { // hack, observe the $(())? via the $()*
            impl From<$t> for MarshalReadError {
                fn from(_err: $t) -> Self { Self::$variant$((_err $($x)*))? }
            }
        };
    }
    impl_from!(str::Utf8Error => Utf8);
    impl_from!(ascii::AsAsciiStrError => Ascii);
    impl_from!(num_enum::TryFromPrimitiveError<Type> => UnknownType);
    impl_from!(bytecode::CodeDeserializeError => Bytecode());

    impl IntoPyException for MarshalReadError {
        fn into_pyexception(self, vm: &VirtualMachine) -> PyBaseExceptionRef {
            match self {
                Self::TooShort => vm.new_exception_msg(
                    vm.ctx.exceptions.eof_error.to_owned(),
                    "marshal data too short".to_owned(),
                ),
                Self::Utf8 => vm.new_value_error("invalid utf8 data".to_owned()),
                Self::Ascii => vm.new_value_error("invalid ascii data".to_owned()),
                Self::UnknownType => {
                    vm.new_value_error("bad marshal data (unknown type code)".to_owned())
                }
                Self::Bytecode(bytecode::CodeDeserializeError::Eof) => vm.new_exception_msg(
                    vm.ctx.exceptions.eof_error.to_owned(),
                    "End of file while deserializing bytecode".to_owned(),
                ),
                Self::Bytecode(_) => {
                    vm.new_value_error("Couldn't deserialize python bytecode".to_owned())
                }
                Self::BufferDiscontiguous => vm.new_buffer_error(
                    "Buffer provided to marshal.loads() is not contiguous".to_owned(),
                ),
            }
        }
    }

    struct ReadBuf<'a>(&'a [u8]);

    impl<'a> ReadBuf<'a> {
        fn read_n(&mut self, n: usize) -> MarshalReadResult<&'a [u8]> {
            if self.0.len() < n {
                return Err(MarshalReadError::TooShort);
            }
            let (buf, rest) = self.0.split_at(n);
            self.0 = rest;
            Ok(buf)
        }
        fn read_n_const<const N: usize>(&mut self) -> MarshalReadResult<&'a [u8; N]> {
            self.read_n(N).map(|buf| buf.try_into().unwrap())
        }

        /// Read the next 4 bytes of a slice, read as u32, pass as usize.
        fn read_size(&mut self) -> MarshalReadResult<usize> {
            Ok(u32::from_le_bytes(*self.read_n_const::<4>()?) as usize)
        }

        fn read_n_prefixed(&mut self) -> MarshalReadResult<&'a [u8]> {
            let len = self.read_size()?;
            self.read_n(len)
        }
    }

    fn load_vec(buf: &mut ReadBuf<'_>, vm: &VirtualMachine) -> MarshalReadResult<Vec<PyObjectRef>> {
        let size = buf.read_size()?;
        let mut vec = Vec::with_capacity(size);
        for _ in 0..size {
            vec.push(load_obj(buf, vm)?);
        }
        Ok(vec)
    }

    #[pyfunction]
    fn loads(pybuffer: PyBuffer, vm: &VirtualMachine) -> MarshalReadResult<PyObjectRef> {
        let buf = pybuffer
            .as_contiguous()
            .ok_or(MarshalReadError::BufferDiscontiguous)?;
        load_obj(&mut ReadBuf(&buf), vm)
    }

    fn load_obj(buf: &mut ReadBuf<'_>, vm: &VirtualMachine) -> MarshalReadResult<PyObjectRef> {
        let [type_indicator] = *buf.read_n_const::<1>()?;
        let typ = Type::try_from(type_indicator).map_err(|_| MarshalReadError::UnknownType)?;
        let obj = match typ {
            Type::True => true.to_pyobject(vm),
            Type::False => false.to_pyobject(vm),
            Type::None => vm.ctx.none(),
            Type::Ellipsis => vm.ctx.ellipsis(),
            Type::Int => {
                let len = i32::from_le_bytes(*buf.read_n_const::<4>()?);
                let sign = if len < 0 { Sign::Minus } else { Sign::Plus };
                let len = len.unsigned_abs() as usize;
                BigInt::from_bytes_le(sign, buf.read_n(len)?).to_pyobject(vm)
            }
            Type::Float => f64::from_le_bytes(*buf.read_n_const::<8>()?).to_pyobject(vm),
            Type::Ascii => {
                let bytes = buf.read_n_prefixed()?;
                ascii::AsciiStr::from_ascii(bytes)?.to_pyobject(vm)
            }
            Type::Unicode => {
                let bytes = buf.read_n_prefixed()?;
                str::from_utf8(bytes)?.to_pyobject(vm)
            }
            Type::List => load_vec(buf, vm)?.to_pyobject(vm),
            Type::Set => {
                let size = buf.read_size()?;
                let set = PySet::new_ref(&vm.ctx);
                for _ in 0..size {
                    // safe to unwrap because builtin types' eq/hash don't error
                    set.add(load_obj(buf, vm)?, vm).unwrap();
                }
                set.into()
            }
            Type::FrozenSet => {
                let size = buf.read_size()?;
                let set = PyFrozenSet::builder(vm);
                for _ in 0..size {
                    // safe to unwrap because builtin types' eq/hash don't error
                    set.add(load_obj(buf, vm)?).unwrap();
                }
                set.build().into()
            }
            Type::Tuple => {
                let elements = load_vec(buf, vm)?;
                vm.ctx.new_tuple(elements).into()
            }
            Type::Dict => {
                let len = buf.read_size()?;
                let dict = vm.ctx.new_dict();
                for _ in 0..len {
                    let key = load_obj(buf, vm)?;
                    let value = load_obj(buf, vm)?;
                    // safe to unwrap because builtin types' eq/hash don't error
                    dict.set_item(key.as_object(), value, vm).unwrap();
                }
                dict.into()
            }
            Type::Bytes => {
                // Following CPython, after marshaling, byte arrays are converted into bytes.
                let bytes = buf.read_n_prefixed()?;
                vm.ctx.new_bytes(bytes.to_vec()).into()
            }
            Type::Code => {
                // If prefix is not identifiable, assume CodeObject, error out if it doesn't match.
                let bytes = buf.read_n_prefixed()?;
                let code = bytecode::CodeObject::from_bytes(bytes)?;
                vm.ctx.new_code(code).into()
            }
        };
        Ok(obj)
    }

    #[pyfunction]
    fn load(f: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let read_res = vm.call_method(&f, "read", ())?;
        let bytes = ArgBytesLike::try_from_object(vm, read_res)?;
        loads(PyBuffer::from(bytes), vm).to_pyresult(vm)
    }
}
