// spell-checker:ignore compresslevel

pub(crate) use _bz2::make_module;

#[pymodule]
mod _bz2 {
    use crate::common::lock::PyMutex;
    use crate::vm::{
        FromArgs, VirtualMachine,
        builtins::{PyBytesRef, PyTypeRef},
        function::{ArgBytesLike, OptionalArg},
        object::{PyPayload, PyResult},
        types::Constructor,
    };
    use bzip2::read::BzDecoder;
    use bzip2::write::BzEncoder;
    use std::io::{Cursor, Read};
    use std::{fmt, io::Write};

    // const BUFSIZ: i32 = 8192;

    struct DecompressorState {
        input_buffer: Vec<u8>,
        // Flag indicating that end-of-stream has been reached.
        eof: bool,
        // Unused data found after the end of stream.
        unused_data: Option<Vec<u8>>,
        needs_input: bool,
    }

    #[pyattr]
    #[pyclass(name = "BZ2Decompressor")]
    #[derive(PyPayload)]
    struct BZ2Decompressor {
        state: PyMutex<DecompressorState>,
    }

    impl fmt::Debug for BZ2Decompressor {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "_bz2.BZ2Decompressor")
        }
    }

    impl Constructor for BZ2Decompressor {
        type Args = ();

        fn py_new(cls: PyTypeRef, _: Self::Args, vm: &VirtualMachine) -> PyResult {
            Self {
                state: PyMutex::new(DecompressorState {
                    eof: false,
                    input_buffer: Vec::new(),
                    unused_data: None,
                    needs_input: true,
                }),
            }
            .into_ref_with_type(vm, cls)
            .map(Into::into)
        }
    }

    #[derive(Debug, FromArgs)]
    struct DecompressArgs {
        #[pyarg(positional)]
        data: ArgBytesLike,
        #[pyarg(any, default = "-1")]
        max_length: i64,
    }

    #[pyclass(with(Constructor))]
    impl BZ2Decompressor {
        #[pymethod]
        fn decompress(&self, args: DecompressArgs, vm: &VirtualMachine) -> PyResult<PyBytesRef> {
            let DecompressArgs { data, max_length } = args;
            let DecompressorState {
                eof,
                input_buffer,
                unused_data,
                needs_input,
            } = &mut *self.state.lock();
            if *eof {
                return Err(vm.new_exception_msg(
                    vm.ctx.exceptions.eof_error.to_owned(),
                    "End of stream already reached".to_owned(),
                ));
            }
            let data_vec = data.borrow_buf().to_vec();
            input_buffer.extend(data_vec);

            // Create a Cursor over the accumulated data.
            let mut cursor = Cursor::new(&input_buffer);
            // Wrap the cursor in a BzDecoder.
            let mut decoder = BzDecoder::new(&mut cursor);
            let mut output = Vec::new();

            // If max_length is nonnegative, read at most that many bytes.
            if max_length >= 0 {
                let mut limited = decoder.by_ref().take(max_length as u64);
                limited
                    .read_to_end(&mut output)
                    .map_err(|e| vm.new_os_error(format!("Decompression error: {}", e)))?;
            } else {
                decoder
                    .read_to_end(&mut output)
                    .map_err(|e| vm.new_os_error(format!("Decompression error: {}", e)))?;
            }

            // Determine how many bytes were consumed from the input.
            let consumed = cursor.position() as usize;
            // Remove the consumed bytes.
            input_buffer.drain(0..consumed);
            unused_data.replace(input_buffer.clone());
            // skrink the vector to save memory
            input_buffer.shrink_to_fit();
            if let Some(v) = unused_data.as_mut() {
                v.shrink_to_fit();
            }

            if *eof {
                *needs_input = false;
            } else {
                *needs_input = input_buffer.is_empty();
            }

            // If the decoder reached end-of-stream (i.e. no more input remains), mark eof.
            if input_buffer.is_empty() {
                *eof = true;
            }

            Ok(vm.ctx.new_bytes(output))
        }

        #[pygetset]
        fn eof(&self) -> bool {
            let state = self.state.lock();
            state.eof
        }

        #[pygetset]
        fn unused_data(&self, vm: &VirtualMachine) -> PyBytesRef {
            let state = self.state.lock();
            match &state.unused_data {
                Some(data) => vm.ctx.new_bytes(data.clone()),
                None => vm.ctx.new_bytes(Vec::new()),
            }
        }

        #[pygetset]
        fn needs_input(&self) -> bool {
            // False if the decompress() method can provide more
            // decompressed data before requiring new uncompressed input.
            let state = self.state.lock();
            state.needs_input
        }

        // TODO: mro()?
    }

    struct CompressorState {
        flushed: bool,
        encoder: Option<BzEncoder<Vec<u8>>>,
    }

    #[pyattr]
    #[pyclass(name = "BZ2Compressor")]
    #[derive(PyPayload)]
    struct BZ2Compressor {
        state: PyMutex<CompressorState>,
    }

    impl fmt::Debug for BZ2Compressor {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "_bz2.BZ2Compressor")
        }
    }

    impl Constructor for BZ2Compressor {
        type Args = (OptionalArg<i32>,);

        fn py_new(cls: PyTypeRef, args: Self::Args, vm: &VirtualMachine) -> PyResult {
            let (compresslevel,) = args;
            let compresslevel = compresslevel.unwrap_or(9);
            let level = match compresslevel {
                valid_level @ 1..=9 => bzip2::Compression::new(valid_level as u32),
                _ => {
                    return Err(
                        vm.new_value_error("compresslevel must be between 1 and 9".to_owned())
                    );
                }
            };

            Self {
                state: PyMutex::new(CompressorState {
                    flushed: false,
                    encoder: Some(BzEncoder::new(Vec::new(), level)),
                }),
            }
            .into_ref_with_type(vm, cls)
            .map(Into::into)
        }
    }

    // TODO: return partial results from compress() instead of returning everything in flush()
    #[pyclass(with(Constructor))]
    impl BZ2Compressor {
        #[pymethod]
        fn compress(&self, data: ArgBytesLike, vm: &VirtualMachine) -> PyResult<PyBytesRef> {
            let mut state = self.state.lock();
            if state.flushed {
                return Err(vm.new_value_error("Compressor has been flushed".to_owned()));
            }

            // let CompressorState { flushed, encoder } = &mut *state;
            let CompressorState { encoder, .. } = &mut *state;

            // TODO: handle Err
            data.with_ref(|input_bytes| encoder.as_mut().unwrap().write_all(input_bytes).unwrap());
            Ok(vm.ctx.new_bytes(Vec::new()))
        }

        #[pymethod]
        fn flush(&self, vm: &VirtualMachine) -> PyResult<PyBytesRef> {
            let mut state = self.state.lock();
            if state.flushed {
                return Err(vm.new_value_error("Repeated call to flush()".to_owned()));
            }

            // let CompressorState { flushed, encoder } = &mut *state;
            let CompressorState { encoder, .. } = &mut *state;

            // TODO: handle Err
            let out = encoder.take().unwrap().finish().unwrap();
            state.flushed = true;
            Ok(vm.ctx.new_bytes(out.to_vec()))
        }
    }
}
