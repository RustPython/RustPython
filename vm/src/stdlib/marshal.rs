pub(crate) use decl::make_module;

#[pymodule(name = "marshal")]
mod decl {
    use crate::{
        builtins::{
            dict::DictContentType, PyByteArray, PyBytes, PyCode, PyDict, PyFloat, PyFrozenSet,
            PyInt, PyList, PySet, PyStr, PyTuple,
        },
        bytecode,
        convert::ToPyObject,
        function::ArgBytesLike,
        object::AsObject,
        protocol::PyBuffer,
        PyObjectRef, PyResult, TryFromObject, VirtualMachine,
    };
    /// TODO
    /// PyBytes: Currently getting recursion error with match_class!
    use ascii::AsciiStr;
    use num_bigint::{BigInt, Sign};
    use std::ops::Deref;
    use std::slice::Iter;

    const STR_BYTE: u8 = b's';
    const INT_BYTE: u8 = b'i';
    const FLOAT_BYTE: u8 = b'f';
    const BOOL_BYTE: u8 = b'b';
    const LIST_BYTE: u8 = b'[';
    const TUPLE_BYTE: u8 = b'(';
    const DICT_BYTE: u8 = b',';
    const SET_BYTE: u8 = b'~';
    const FROZEN_SET_BYTE: u8 = b'<';
    const BYTE_ARRAY: u8 = b'>';

    /// Safely convert usize to 4 le bytes
    fn size_to_bytes(x: usize, vm: &VirtualMachine) -> PyResult<[u8; 4]> {
        // For marshalling we want to convert lengths to bytes. To save space
        // we limit the size to u32 to keep marshalling smaller.
        match u32::try_from(x) {
            Ok(n) => Ok(n.to_le_bytes()),
            Err(_) => {
                Err(vm.new_value_error("Size exceeds 2^32 capacity for marshaling.".to_owned()))
            }
        }
    }

    /// Dumps a iterator of objects into binary vector.
    fn dump_list(pyobjs: Iter<PyObjectRef>, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        let mut byte_list = size_to_bytes(pyobjs.len(), vm)?.to_vec();
        // For each element, dump into binary, then add its length and value.
        for element in pyobjs {
            let element_bytes: Vec<u8> = _dumps(element.clone(), vm)?;
            byte_list.extend(size_to_bytes(element_bytes.len(), vm)?);
            byte_list.extend(element_bytes)
        }
        Ok(byte_list)
    }

    /// Dumping helper function to turn a value into bytes.
    fn _dumps(value: PyObjectRef, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        let r = match_class!(match value {
            pyint @ PyInt => {
                if pyint.class().is(vm.ctx.types.bool_type) {
                    let (_, mut bool_bytes) = pyint.as_bigint().to_bytes_le();
                    bool_bytes.push(BOOL_BYTE);
                    bool_bytes
                } else {
                    let (sign, mut int_bytes) = pyint.as_bigint().to_bytes_le();
                    let sign_byte = match sign {
                        Sign::Minus => b'-',
                        Sign::NoSign => b'0',
                        Sign::Plus => b'+',
                    };
                    // Return as [TYPE, SIGN, uint bytes]
                    int_bytes.insert(0, sign_byte);
                    int_bytes.push(INT_BYTE);
                    int_bytes
                }
            }
            pyfloat @ PyFloat => {
                let mut float_bytes = pyfloat.to_f64().to_le_bytes().to_vec();
                float_bytes.push(FLOAT_BYTE);
                float_bytes
            }
            pystr @ PyStr => {
                let mut str_bytes = pystr.as_str().as_bytes().to_vec();
                str_bytes.push(STR_BYTE);
                str_bytes
            }
            pylist @ PyList => {
                let pylist_items = pylist.borrow_vec();
                let mut list_bytes = dump_list(pylist_items.iter(), vm)?;
                list_bytes.push(LIST_BYTE);
                list_bytes
            }
            pyset @ PySet => {
                let elements = pyset.elements();
                let mut set_bytes = dump_list(elements.iter(), vm)?;
                set_bytes.push(SET_BYTE);
                set_bytes
            }
            pyfrozen @ PyFrozenSet => {
                let elements = pyfrozen.elements();
                let mut fset_bytes = dump_list(elements.iter(), vm)?;
                fset_bytes.push(FROZEN_SET_BYTE);
                fset_bytes
            }
            pytuple @ PyTuple => {
                let mut tuple_bytes = dump_list(pytuple.iter(), vm)?;
                tuple_bytes.push(TUPLE_BYTE);
                tuple_bytes
            }
            pydict @ PyDict => {
                let key_value_pairs = pydict._as_dict_inner().clone().as_kvpairs();
                // Converts list of tuples to PyObjectRefs of tuples
                let elements: Vec<PyObjectRef> = key_value_pairs
                    .into_iter()
                    .map(|(k, v)| PyTuple::new_ref(vec![k, v], &vm.ctx).to_pyobject(vm))
                    .collect();
                // Converts list of tuples to list, dump into binary
                let mut dict_bytes = dump_list(elements.iter(), vm)?;
                dict_bytes.push(LIST_BYTE);
                dict_bytes.push(DICT_BYTE);
                dict_bytes
            }
            pybyte_array @ PyByteArray => {
                let mut pybytes = pybyte_array.borrow_buf_mut();
                pybytes.push(BYTE_ARRAY);
                pybytes.deref().to_owned()
            }
            co @ PyCode => {
                // Code is default, doesn't have prefix.
                co.code.map_clone_bag(&bytecode::BasicBag).to_bytes()
            }
            _ => {
                return Err(vm.new_not_implemented_error(
                    "TODO: not implemented yet or marshal unsupported type".to_owned(),
                ));
            }
        });
        Ok(r)
    }

    #[pyfunction]
    fn dumps(value: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyBytes> {
        Ok(PyBytes::from(_dumps(value, vm)?))
    }

    #[pyfunction]
    fn dump(value: PyObjectRef, f: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let dumped = dumps(value, vm)?;
        vm.call_method(&f, "write", (dumped,))?;
        Ok(())
    }

    /// Read the next 4 bytes of a slice, read as u32, pass as usize.
    /// Returns the rest of buffer with the value.
    fn eat_length<'a>(bytes: &'a [u8], vm: &VirtualMachine) -> PyResult<(usize, &'a [u8])> {
        let (u32_bytes, rest) = bytes.split_at(4);
        let length = u32::from_le_bytes(u32_bytes.try_into().map_err(|_| {
            vm.new_value_error("Could not read u32 size from byte array".to_owned())
        })?);
        Ok((length as usize, rest))
    }

    /// Reads next element from a python list. First by getting element size
    /// then by building a pybuffer and "loading" the pyobject.
    /// Returns rest of buffer with object.
    fn next_element_of_list<'a>(
        buf: &'a [u8],
        vm: &VirtualMachine,
    ) -> PyResult<(PyObjectRef, &'a [u8])> {
        let (element_length, element_and_rest) = eat_length(buf, vm)?;
        let (element_buff, rest) = element_and_rest.split_at(element_length);
        let pybuffer = PyBuffer::from_byte_vector(element_buff.to_vec(), vm);
        Ok((loads(pybuffer, vm)?, rest))
    }

    /// Reads a list (or tuple) from a buffer.
    fn read_list(buf: &[u8], vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
        let (expected_array_len, mut buffer) = eat_length(buf, vm)?;
        let mut elements: Vec<PyObjectRef> = Vec::new();
        while !buffer.is_empty() {
            let (element, rest_of_buffer) = next_element_of_list(buffer, vm)?;
            elements.push(element);
            buffer = rest_of_buffer;
        }
        debug_assert!(expected_array_len == elements.len());
        Ok(elements)
    }

    /// Builds a PyDict from iterator of tuple objects
    pub fn from_tuples(iterable: Iter<PyObjectRef>, vm: &VirtualMachine) -> PyResult<PyDict> {
        let dict = DictContentType::default();
        for elem in iterable {
            let items = match_class!(match elem.clone() {
                pytuple @ PyTuple => pytuple.to_vec(),
                _ =>
                    return Err(vm.new_value_error(
                        "Couldn't unmarshal key:value pair of dictionary".to_owned()
                    )),
            });
            // Marshalled tuples are always in format key:value.
            dict.insert(vm, &**items.get(0).unwrap(), items.get(1).unwrap().clone())?;
        }
        Ok(PyDict::from_entries(dict))
    }

    #[pyfunction]
    fn loads(pybuffer: PyBuffer, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let full_buff = pybuffer.as_contiguous().ok_or_else(|| {
            vm.new_buffer_error("Buffer provided to marshal.loads() is not contiguous".to_owned())
        })?;
        let (type_indicator, buf) = full_buff.split_last().ok_or_else(|| {
            vm.new_exception_msg(
                vm.ctx.exceptions.eof_error.to_owned(),
                "EOF where object expected.".to_owned(),
            )
        })?;
        match *type_indicator {
            BOOL_BYTE => Ok((buf[0] != 0).to_pyobject(vm)),
            INT_BYTE => {
                let (sign_byte, uint_bytes) = buf
                    .split_first()
                    .ok_or_else(|| vm.new_value_error("EOF where object expected.".to_owned()))?;
                let sign = match sign_byte {
                    b'-' => Sign::Minus,
                    b'0' => Sign::NoSign,
                    b'+' => Sign::Plus,
                    _ => {
                        return Err(vm.new_value_error(
                            "Unknown sign byte when trying to unmarshal integer".to_owned(),
                        ))
                    }
                };
                let pyint = BigInt::from_bytes_le(sign, uint_bytes);
                Ok(pyint.to_pyobject(vm))
            }
            FLOAT_BYTE => {
                let number = f64::from_le_bytes(match buf[..].try_into() {
                    Ok(byte_array) => byte_array,
                    Err(e) => {
                        return Err(vm.new_value_error(format!(
                            "Expected float, could not load from bytes. {}",
                            e
                        )))
                    }
                });
                let pyfloat = PyFloat::from(number);
                Ok(pyfloat.to_pyobject(vm))
            }
            STR_BYTE => {
                let pystr = PyStr::from(match AsciiStr::from_ascii(buf) {
                    Ok(ascii_str) => ascii_str,
                    Err(e) => {
                        return Err(
                            vm.new_value_error(format!("Cannot unmarshal bytes to string, {}", e))
                        )
                    }
                });
                Ok(pystr.to_pyobject(vm))
            }
            LIST_BYTE => {
                let elements = read_list(buf, vm)?;
                Ok(elements.to_pyobject(vm))
            }
            SET_BYTE => {
                let elements = read_list(buf, vm)?;
                let set = PySet::new_ref(&vm.ctx);
                for element in elements {
                    set.add(element, vm)?;
                }
                Ok(set.to_pyobject(vm))
            }
            FROZEN_SET_BYTE => {
                let elements = read_list(buf, vm)?;
                let set = PyFrozenSet::from_iter(vm, elements.into_iter())?;
                Ok(set.to_pyobject(vm))
            }
            TUPLE_BYTE => {
                let elements = read_list(buf, vm)?;
                let pytuple = PyTuple::new_ref(elements, &vm.ctx).to_pyobject(vm);
                Ok(pytuple)
            }
            DICT_BYTE => {
                let pybuffer = PyBuffer::from_byte_vector(buf[..].to_vec(), vm);
                let pydict = match_class!(match loads(pybuffer, vm)? {
                    pylist @ PyList => from_tuples(pylist.borrow_vec().iter(), vm)?,
                    _ =>
                        return Err(vm.new_value_error("Couldn't unmarshal dicitionary.".to_owned())),
                });
                Ok(pydict.to_pyobject(vm))
            }
            BYTE_ARRAY => {
                // Following CPython, after marshaling, byte arrays are converted into bytes.
                let byte_array = PyBytes::from(buf[..].to_vec());
                Ok(byte_array.to_pyobject(vm))
            }
            _ => {
                // If prefix is not identifiable, assume CodeObject, error out if it doesn't match.
                let code = bytecode::CodeObject::from_bytes(&full_buff).map_err(|e| match e {
                    bytecode::CodeDeserializeError::Eof => vm.new_exception_msg(
                        vm.ctx.exceptions.eof_error.to_owned(),
                        "End of file while deserializing bytecode".to_owned(),
                    ),
                    _ => vm.new_value_error("Couldn't deserialize python bytecode".to_owned()),
                })?;
                Ok(vm.ctx.new_code(code).into())
            }
        }
    }

    #[pyfunction]
    fn load(f: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let read_res = vm.call_method(&f, "read", ())?;
        let bytes = ArgBytesLike::try_from_object(vm, read_res)?;
        loads(PyBuffer::from(bytes), vm)
    }
}
