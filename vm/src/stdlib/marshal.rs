pub(crate) use decl::make_module;

#[pymodule(name = "marshal")]
mod decl {
    /// TODO add support for Booleans, Sets, etc
    use ascii::AsciiStr;
    use num_bigint::{BigInt, Sign};
    use std::ops::Deref;
    use std::slice::Iter;

    use crate::{
        builtins::{
            dict::DictContentType, PyBytes, PyCode, PyDict, PyFloat, PyInt, PyList, PyStr, PyTuple,
        },
        bytecode,
        common::borrow::BorrowedValue,
        function::{ArgBytesLike, IntoPyObject},
        protocol::PyBuffer,
        PyObjectRef, PyResult, TryFromObject, VirtualMachine,
    };

    const STR_BYTE: u8 = b's';
    const INT_BYTE: u8 = b'i';
    const FLOAT_BYTE: u8 = b'f';
    const LIST_BYTE: u8 = b'[';
    const TUPLE_BYTE: u8 = b'(';
    const DICT_BYTE: u8 = b',';

    /// Safely convert usize to 4 le bytes
    fn size_to_bytes(x: usize, vm: &VirtualMachine) -> PyResult<[u8; 4]> {
        // For marshalling we want to convert lengths to bytes. To save space
        // we limit the size to u32 to keep marshalling smaller.
        match u32::try_from(x) {
            Ok(n) => Ok(n.to_le_bytes()),
            Err(_) => {
                Err(vm.new_value_error("Size exceeds 2^32 capacity for marshalling.".to_owned()))
            }
        }
    }

    /// Dumps a iterator of objects into binary vector.
    fn dump_list(pyobjs: Iter<PyObjectRef>, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        let mut byte_list = size_to_bytes(pyobjs.len(), vm)?.to_vec();
        // For each element, dump into binary, then add its length and value.
        for element in pyobjs {
            let element_bytes: PyBytes = dumps(element.clone(), vm)?;
            byte_list.extend(size_to_bytes(element_bytes.len(), vm)?);
            byte_list.extend_from_slice(element_bytes.deref())
        }
        Ok(byte_list)
    }

    #[pyfunction]
    fn dumps(value: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyBytes> {
        let r = match_class!(match value {
            pyint @ PyInt => {
                let (sign, uint_bytes) = pyint.as_bigint().to_bytes_le();
                let sign_byte = match sign {
                    Sign::Minus => b'-',
                    Sign::NoSign => b'0',
                    Sign::Plus => b'+',
                };
                // Return as [TYPE, SIGN, uint bytes]
                PyBytes::from([vec![INT_BYTE, sign_byte], uint_bytes].concat())
            }
            pyfloat @ PyFloat => {
                let mut float_bytes = pyfloat.to_f64().to_le_bytes().to_vec();
                float_bytes.insert(0, FLOAT_BYTE);
                PyBytes::from(float_bytes)
            }
            pystr @ PyStr => {
                let mut str_bytes = pystr.as_str().as_bytes().to_vec();
                str_bytes.insert(0, STR_BYTE);
                PyBytes::from(str_bytes)
            }
            pylist @ PyList => {
                let pylist_items = pylist.borrow_vec();
                let mut list_bytes = dump_list(pylist_items.iter(), vm)?;
                list_bytes.insert(0, LIST_BYTE);
                PyBytes::from(list_bytes)
            }
            pytuple @ PyTuple => {
                let mut tuple_bytes = dump_list(pytuple.as_slice().iter(), vm)?;
                tuple_bytes.insert(0, TUPLE_BYTE);
                PyBytes::from(tuple_bytes)
            }
            pydict @ PyDict => {
                let key_value_pairs = pydict._as_dict_inner().clone().as_kvpairs();
                // Converts list of tuples to PyObjectRefs of tuples
                let elements: Vec<PyObjectRef> = key_value_pairs
                    .into_iter()
                    .map(|(k, v)| {
                        PyTuple::new_ref(vec![k, v], &vm.ctx).into_pyobject(vm)
                    })
                    .collect();
                // Converts list of tuples to list, dump into binary
                let mut dict_bytes = dump_list(elements.iter(), vm)?;
                dict_bytes.insert(0, LIST_BYTE);
                dict_bytes.insert(0, DICT_BYTE);
                PyBytes::from(dict_bytes)
            }
            co @ PyCode => {
                // Code is default, doesn't have prefix.
                let code_bytes = co.code.map_clone_bag(&bytecode::BasicBag).to_bytes();
                PyBytes::from(code_bytes)
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
    fn dump(value: PyObjectRef, f: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let dumped = dumps(value, vm)?;
        vm.call_method(&f, "write", (dumped,))?;
        Ok(())
    }

    /// Read the next 4 bytes of a slice, convert to u32.
    /// Side effect: increasing position pointer by 4.
    fn eat_u32(bytes: &[u8], position: &mut usize, vm: &VirtualMachine) -> PyResult<u32> {
        let length_as_u32 =
            u32::from_le_bytes(match bytes[*position..(*position + 4)].try_into() {
                Ok(length_as_u32) => length_as_u32,
                Err(_) => {
                    return Err(
                        vm.new_buffer_error("Could not read u32 size from byte array".to_owned())
                    )
                }
            });
        *position += 4;
        Ok(length_as_u32)
    }

    /// Reads next element from a python list. First by getting element size
    /// then by building a pybuffer and "loading" the pyobject.
    /// Moves the position pointer past the element.
    fn next_element_of_list(
        buf: &BorrowedValue<[u8]>,
        position: &mut usize,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        // Read size of the current element from buffer.
        let element_length = eat_u32(buf, position, vm)? as usize;
        // Create pybuffer consisting of the data in the next element.
        let pybuffer =
            PyBuffer::from_byte_vector(buf[*position..(*position + element_length)].to_vec(), vm);
        // Move position pointer past element.
        *position += element_length;
        // Return marshalled element.
        loads(pybuffer, vm)
    }

    /// Reads a list (or tuple) from a buffer.
    fn read_list(buf: &BorrowedValue<[u8]>, vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
        let mut position = 1;
        let expected_array_len = eat_u32(buf, &mut position, vm)? as usize;
        // Read each element in list, incrementing position pointer to reflect position in the buffer.
        let mut elements: Vec<PyObjectRef> = Vec::new();
        while position < buf.len() {
            elements.push(next_element_of_list(buf, &mut position, vm)?);
        }
        debug_assert!(expected_array_len == elements.len());
        debug_assert!(buf.len() == position);
        Ok(elements)
    }

    /// Builds a PyDict from iterator of tuple objects
    pub fn from_tuples(iterable: Iter<PyObjectRef>, vm: &VirtualMachine) -> PyResult<PyDict> {
        let dict = DictContentType::default();
        for elem in iterable {
            let items = match_class!(match elem.clone() {
                pytuple @ PyTuple => pytuple.as_slice().to_vec(),
                _ =>
                    return Err(vm.new_value_error(
                        "Couldn't unmarshal key:value pair of dictionary".to_owned()
                    )),
            });
            // Marshalled tuples are always in format key:value.
            dict.insert(
                vm,
                items.get(0).unwrap().clone(),
                items.get(1).unwrap().clone(),
            )?;
        }
        Ok(PyDict::from_entries(dict))
    }

    #[pyfunction]
    fn loads(pybuffer: PyBuffer, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let buf = &pybuffer.as_contiguous().ok_or_else(|| {
            vm.new_buffer_error("Buffer provided to marshal.loads() is not contiguous".to_owned())
        })?;
        match buf[0] {
            INT_BYTE => {
                let sign = match buf[1] {
                    b'-' => Sign::Minus,
                    b'0' => Sign::NoSign,
                    b'+' => Sign::Plus,
                    _ => {
                        return Err(vm.new_value_error(
                            "Unknown sign byte when trying to unmarshal integer".to_owned(),
                        ))
                    }
                };
                let pyint = BigInt::from_bytes_le(sign, &buf[2..buf.len()]);
                Ok(pyint.into_pyobject(vm))
            }
            FLOAT_BYTE => {
                let number = f64::from_le_bytes(match buf[1..buf.len()].try_into() {
                    Ok(byte_array) => byte_array,
                    Err(e) => {
                        return Err(vm.new_value_error(format!(
                            "Expected float, could not load from bytes. {}",
                            e
                        )))
                    }
                });
                let pyfloat = PyFloat::from(number);
                Ok(pyfloat.into_pyobject(vm))
            }
            STR_BYTE => {
                let pystr = PyStr::from(match AsciiStr::from_ascii(&buf[1..buf.len()]) {
                    Ok(ascii_str) => ascii_str,
                    Err(e) => {
                        return Err(
                            vm.new_value_error(format!("Cannot unmarshal bytes to string, {}", e))
                        )
                    }
                });
                Ok(pystr.into_pyobject(vm))
            }
            LIST_BYTE => {
                let elements = read_list(buf, vm)?;
                Ok(elements.into_pyobject(vm))
            }
            TUPLE_BYTE => {
                let elements = read_list(buf, vm)?;
                let pytuple = PyTuple::new_ref(elements, &vm.ctx).into_pyobject(vm);
                Ok(pytuple)
            }
            DICT_BYTE => {
                let pybuffer = PyBuffer::from_byte_vector(buf[1..buf.len()].to_vec(), vm);
                let pydict = match_class!(match loads(pybuffer, vm)? {
                    pylist @ PyList => from_tuples(pylist.borrow_vec().iter(), vm)?,
                    _ =>
                        return Err(vm.new_value_error("Couldn't unmarshal dicitionary.".to_owned())),
                });
                Ok(pydict.into_pyobject(vm))
            }
            _ => {
                // If prefix is not identifiable, assume CodeObject, error out if it doesn't match.
                let code = bytecode::CodeObject::from_bytes(&buf).map_err(|e| match e {
                    bytecode::CodeDeserializeError::Eof => vm.new_exception_msg(
                        vm.ctx.exceptions.eof_error.clone(),
                        "End of file while deserializing bytecode".to_owned(),
                    ),
                    _ => vm.new_value_error("Couldn't deserialize python bytecode".to_owned()),
                })?;
                Ok(PyCode {
                    code: vm.map_codeobj(code),
                }
                .into_pyobject(vm))
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
