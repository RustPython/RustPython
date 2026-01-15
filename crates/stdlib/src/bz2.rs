// spell-checker:ignore compresslevel

pub(crate) use _bz2::make_module;

#[pymodule]
mod _bz2 {
    use crate::compression::{
        DecompressArgs, DecompressError, DecompressState, DecompressStatus, Decompressor,
    };
    use crate::vm::{
        Py, VirtualMachine,
        builtins::{PyBytesRef, PyType},
        common::lock::PyMutex,
        function::{ArgBytesLike, OptionalArg},
        object::PyResult,
        types::Constructor,
    };
    use alloc::fmt;
    use bzip2::{Decompress, Status, write::BzEncoder};
    use rustpython_vm::convert::ToPyException;
    use std::io::Write;

    const BUFSIZ: usize = 8192;

    #[pyattr]
    #[pyclass(name = "BZ2Decompressor")]
    #[derive(PyPayload)]
    struct BZ2Decompressor {
        state: PyMutex<DecompressState<Decompress>>,
    }

    impl Decompressor for Decompress {
        type Flush = ();
        type Status = Status;
        type Error = bzip2::Error;

        fn total_in(&self) -> u64 {
            self.total_in()
        }
        fn decompress_vec(
            &mut self,
            input: &[u8],
            output: &mut Vec<u8>,
            (): Self::Flush,
        ) -> Result<Self::Status, Self::Error> {
            self.decompress_vec(input, output)
        }
    }

    impl DecompressStatus for Status {
        fn is_stream_end(&self) -> bool {
            *self == Self::StreamEnd
        }
    }

    impl fmt::Debug for BZ2Decompressor {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "_bz2.BZ2Decompressor")
        }
    }

    impl Constructor for BZ2Decompressor {
        type Args = ();

        fn py_new(_cls: &Py<PyType>, _args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            Ok(Self {
                state: PyMutex::new(DecompressState::new(Decompress::new(false), vm)),
            })
        }
    }

    #[pyclass(with(Constructor))]
    impl BZ2Decompressor {
        #[pymethod]
        fn decompress(&self, args: DecompressArgs, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            let max_length = args.max_length();
            let data = &*args.data();

            let mut state = self.state.lock();
            state
                .decompress(data, max_length, BUFSIZ, vm)
                .map_err(|e| match e {
                    DecompressError::Decompress(err) => vm.new_os_error(err.to_string()),
                    DecompressError::Eof(err) => err.to_pyexception(vm),
                })
        }

        #[pygetset]
        fn eof(&self) -> bool {
            self.state.lock().eof()
        }

        #[pygetset]
        fn unused_data(&self) -> PyBytesRef {
            self.state.lock().unused_data()
        }

        #[pygetset]
        fn needs_input(&self) -> bool {
            // False if the decompress() method can provide more
            // decompressed data before requiring new uncompressed input.
            self.state.lock().needs_input()
        }

        #[pymethod(name = "__reduce__")]
        fn reduce(&self, vm: &VirtualMachine) -> PyResult<()> {
            Err(vm.new_type_error("cannot pickle '_bz2.BZ2Decompressor' object"))
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

        fn py_new(
            _cls: &Py<PyType>,
            (compresslevel,): Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult<Self> {
            // TODO: seriously?
            // compresslevel.unwrap_or(bzip2::Compression::best().level().try_into().unwrap());
            let compresslevel = compresslevel.unwrap_or(9);
            let level = match compresslevel {
                valid_level @ 1..=9 => bzip2::Compression::new(valid_level as u32),
                _ => {
                    return Err(vm.new_value_error("compresslevel must be between 1 and 9"));
                }
            };

            Ok(Self {
                state: PyMutex::new(CompressorState {
                    flushed: false,
                    encoder: Some(BzEncoder::new(Vec::new(), level)),
                }),
            })
        }
    }

    // TODO: return partial results from compress() instead of returning everything in flush()
    #[pyclass(with(Constructor))]
    impl BZ2Compressor {
        #[pymethod]
        fn compress(&self, data: ArgBytesLike, vm: &VirtualMachine) -> PyResult<PyBytesRef> {
            let mut state = self.state.lock();
            if state.flushed {
                return Err(vm.new_value_error("Compressor has been flushed"));
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
                return Err(vm.new_value_error("Repeated call to flush()"));
            }

            // let CompressorState { flushed, encoder } = &mut *state;
            let CompressorState { encoder, .. } = &mut *state;

            // TODO: handle Err
            let out = encoder.take().unwrap().finish().unwrap();
            state.flushed = true;
            Ok(vm.ctx.new_bytes(out.to_vec()))
        }

        #[pymethod(name = "__reduce__")]
        fn reduce(&self, vm: &VirtualMachine) -> PyResult<()> {
            Err(vm.new_type_error("cannot pickle '_bz2.BZ2Compressor' object"))
        }
    }
}
