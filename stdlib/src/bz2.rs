pub(crate) use _bz2::make_module;

#[pymodule]
mod _bz2 {
    use crate::common::lock::PyMutex;
    use crate::vm::{
        builtins::{PyBytesRef, PyTypeRef},
        function::{ArgBytesLike, OptionalArg},
        object::{PyPayload, PyResult},
        types::Constructor,
        VirtualMachine,
    };
    use bzip2::{write::BzEncoder, Decompress, Status};
    use std::{fmt, io::Write};

    // const BUFSIZ: i32 = 8192;

    struct DecompressorState {
        decoder: Decompress,
        eof: bool,
        needs_input: bool,
        // input_buffer: Vec<u8>,
        // output_buffer: Vec<u8>,
    }

    #[pyattr]
    #[pyclass(name = "BZ2Decompressor")]
    #[derive(PyPayload)]
    struct BZ2Decompressor {
        state: PyMutex<DecompressorState>,
    }

    impl fmt::Debug for BZ2Decompressor {
        fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
            write!(f, "_bz2.BZ2Decompressor")
        }
    }

    impl Constructor for BZ2Decompressor {
        type Args = ();

        fn py_new(cls: PyTypeRef, _: Self::Args, vm: &VirtualMachine) -> PyResult {
            Self {
                state: PyMutex::new(DecompressorState {
                    decoder: Decompress::new(false),
                    eof: false,
                    needs_input: true,
                    // input_buffer: Vec::new(),
                    // output_buffer: Vec::new(),
                }),
            }
            .into_ref_with_type(vm, cls)
            .map(Into::into)
        }
    }

    #[pyclass(with(Constructor))]
    impl BZ2Decompressor {
        #[pymethod]
        fn decompress(
            &self,
            data: ArgBytesLike,
            // TODO: PyIntRef
            max_length: OptionalArg<i32>,
            vm: &VirtualMachine,
        ) -> PyResult<PyBytesRef> {
            let max_length = max_length.unwrap_or(-1);
            if max_length >= 0 {
                return Err(vm.new_not_implemented_error(
                    "the max_value argument is not implemented yet".to_owned(),
                ));
            }
            // let max_length = if max_length < 0 || max_length >= BUFSIZ {
            //     BUFSIZ
            // } else {
            //     max_length
            // };

            let mut state = self.state.lock();
            let DecompressorState {
                decoder,
                eof,
                ..
                // needs_input,
                // input_buffer,
                // output_buffer,
            } = &mut *state;

            if *eof {
                return Err(vm.new_exception_msg(
                    vm.ctx.exceptions.eof_error.to_owned(),
                    "End of stream already reached".to_owned(),
                ));
            }

            // data.with_ref(|data| input_buffer.extend(data));

            // If max_length is negative:
            // read the input X bytes at a time, compress it and append it to output.
            // Once you're out of input, setting needs_input to true and return the
            // output as bytes.
            //
            // TODO:
            // If max_length is non-negative:
            // Read the input X bytes at a time, compress it and append it to
            // the output. If output reaches `max_length` in size, return
            // it (up to max_length), and store the rest of the output
            // for later.

            // TODO: arbitrary choice, not the right way to do it.
            let mut buf = Vec::with_capacity(data.len() * 32);

            let before = decoder.total_in();
            let res = data.with_ref(|data| decoder.decompress_vec(data, &mut buf));
            let _written = (decoder.total_in() - before) as usize;

            let res = match res {
                Ok(x) => x,
                // TODO: error message
                _ => return Err(vm.new_os_error("Invalid data stream".to_owned())),
            };

            if res == Status::StreamEnd {
                *eof = true;
            }
            Ok(vm.ctx.new_bytes(buf.to_vec()))
        }

        #[pyproperty]
        fn eof(&self) -> bool {
            let state = self.state.lock();
            state.eof
        }

        #[pyproperty]
        fn unused_data(&self, vm: &VirtualMachine) -> PyBytesRef {
            // Data found after the end of the compressed stream.
            // If this attribute is accessed before the end of the stream
            // has been reached, its value will be b''.
            vm.ctx.new_bytes(b"".to_vec())
            // alternatively, be more honest:
            // Err(vm.new_not_implemented_error(
            //     "unused_data isn't implemented yet".to_owned(),
            // ))
            //
            // TODO
            // let state = self.state.lock();
            // if state.eof {
            //     vm.ctx.new_bytes(state.input_buffer.to_vec())
            // else {
            //     vm.ctx.new_bytes(b"".to_vec())
            // }
        }

        #[pyproperty]
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
        fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
            write!(f, "_bz2.BZ2Compressor")
        }
    }

    impl Constructor for BZ2Compressor {
        type Args = (OptionalArg<i32>,);

        fn py_new(cls: PyTypeRef, args: Self::Args, vm: &VirtualMachine) -> PyResult {
            let (compresslevel,) = args;
            // TODO: seriously?
            // compresslevel.unwrap_or(bzip2::Compression::best().level().try_into().unwrap());
            let compresslevel = compresslevel.unwrap_or(9);
            let level = match compresslevel {
                valid_level @ 1..=9 => bzip2::Compression::new(valid_level as u32),
                _ => {
                    return Err(
                        vm.new_value_error("compresslevel must be between 1 and 9".to_owned())
                    )
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
